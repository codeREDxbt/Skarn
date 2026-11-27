//! The filesystem tools (ARCHITECTURE.md §17.1) — Phase 5b's file.
//!
//! Read tools (`read_file`, `list_dir`, `fuzzy_find`, `search_files`)
//! execute immediately and return their output. Write tools (`write_file`,
//! `edit_file`, `delete_file`) are **planned, never executed, here**: the
//! router turns the plan into an [`ApprovalCard`](crate::agent::ApprovalCard)
//! and the loop parks (§17.4); only [`apply_write`] / [`apply_edit`] /
//! [`apply_delete`] — called by the router's `execute_approved` after the
//! user's decision — touch disk. A Ready outcome for a side-effecting tool
//! would be a security bug, so planning and application are separate
//! functions with separate tests.
//!
//! Guards in this file (each has its own test, citing its §):
//! - **Path rooting** (§17.2): [`resolve_rooted`].
//! - **Read cap** (§17.2 "Bounded I/O"): [`MAX_READ_BYTES`] on reads.
//! - **Write cap** (§17.2 / brief): [`MAX_READ_BYTES`] on writes — an
//!   approved write larger than 4MB is refused.
//! - **Output truncation** (§17.2): listings are capped at
//!   [`super::MAX_OUTPUT_BYTES`] with a visible notice.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::chat_types::ToolResult;

use super::MAX_OUTPUT_BYTES;

/// Compare-and-swap precondition captured at tool planning time to eliminate
/// TOCTOU drift between approval card creation and disk apply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedFilePrecondition {
    pub existed: bool,
    pub sha256: Option<String>,
}

/// Result of a CAS filesystem mutation, returning both the model-visible tool
/// result and the exact before/after bytes for [`skarn_decision::TurnCheckpointStore`].
#[derive(Clone, Debug)]
pub(crate) struct CasMutationResult {
    pub tool_result: ToolResult,
    pub file_change: Option<skarn_decision::TurnFileChange>,
}

pub(crate) fn capture_file_precondition(resolved: &Path) -> Result<ExpectedFilePrecondition, String> {
    match std::fs::read(resolved) {
        Ok(bytes) => Ok(ExpectedFilePrecondition {
            existed: true,
            sha256: Some(skarn_decision::sha256_bytes(&bytes)),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(ExpectedFilePrecondition {
            existed: false,
            sha256: None,
        }),
        Err(err) => Err(format!("cannot read `{}`: {err}", resolved.display())),
    }
}

fn verify_cas_precondition(
    resolved: &Path,
    rel: &str,
    expected: Option<&ExpectedFilePrecondition>,
) -> Result<Option<Vec<u8>>, String> {
    let live = match std::fs::read(resolved) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(format!("cannot read `{rel}`: {err}")),
    };
    if let Some(exp) = expected {
        match (&live, exp.existed) {
            (Some(bytes), true) => {
                let live_sha = skarn_decision::sha256_bytes(bytes);
                if exp.sha256.as_deref() != Some(live_sha.as_str()) {
                    return Err(format!(
                        "`{rel}` changed on disk after planning/approval (SHA-256 mismatch) — aborting CAS mutation"
                    ));
                }
            }
            (Some(_), false) => {
                return Err(format!(
                    "`{rel}` was created on disk after planning/approval — aborting CAS mutation"
                ));
            }
            (None, true) => {
                return Err(format!(
                    "`{rel}` was deleted from disk after planning/approval — aborting CAS mutation"
                ));
            }
            (None, false) => {}
        }
    }
    Ok(live)
}

/// Heuristic for paths that commonly contain credentials. This is not a sandbox.
pub(crate) fn is_secret_path(path: &str) -> bool {
    path.replace('\\', "/").split('/').any(|part| {
        let name = part.to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name == ".npmrc"
            || name == ".pypirc"
            || name.contains("credential")
            || name.contains("secret")
            || name.contains("password")
    })
}

fn reject_secret(root: &Path, resolved: &Path) -> Result<(), String> {
    let rel = rel_display(root, resolved);
    if is_secret_path(&rel) {
        Err(format!("refusing to expose secret-looking path `{rel}`"))
    } else {
        Ok(())
    }
}

/// §17.2 "Bounded I/O": a **full** read is ≤4MB — a larger file is refused,
/// and the error names the cap and the actual size. A read with
/// `start_line`/`end_line` streams only that window ([`read_line_slice`]),
/// so it is allowed past the cap; the returned slice is capped at this many
/// bytes too.
pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

/// §17.1 `edit_file`: hits returned before the "show more" notice.
const MAX_SEARCH_HITS: usize = 200;

/// Cap on the diff body rendered into a diff card (UI_SPEC.md §4.2: the
/// card scrolls; a full-file monster diff is truncated with a notice).
const MAX_DIFF_LINES: usize = 4_000;

/// The unified-diff context (lines of unchanged context around each hunk).
const DIFF_CONTEXT: usize = 3;

// -----------------------------------------------------------------------------
// §17.2 path rooting
// -----------------------------------------------------------------------------

/// **§17.2 guard — path rooting.** Resolve the user/model-supplied `user`
/// against the workspace `root`, refusing every escape:
///
/// - `..` traversal that would leave the root (also when it pops past the
///   path's own drive root);
/// - absolute paths outside the root (absolute paths *inside* the root are
///   accepted — §17.2 only rejects escape);
/// - symlink/junction escapes: the deepest *existing* ancestor is
///   canonicalized and re-checked, so a link inside the workspace pointing
///   outside is refused (the target's canonical path fails containment).
///
/// Rejections are `Err(String)`s the router wraps into a tool-result error
/// the model can see and correct. Escaping the root requires the user's
/// per-session opt-in, which does not exist in Lite.
///
/// Returns the **canonical** (symlink-free) path — on Windows this is a
/// `\\?\`-prefixed verbatim path; hand [`de_verbatim`] to processes and
/// [`rel_display`] to the UI.
pub(crate) fn resolve_rooted(root: &Path, user: &str) -> Result<PathBuf, String> {
    let escape = || {
        format!(
            "path `{user}` escapes the workspace root — every path must stay inside {}",
            root.display()
        )
    };

    if user.trim().is_empty() {
        return Err("path must not be empty (use `.` for the workspace root)".to_owned());
    }

    let raw = Path::new(user);
    // An absolute argument replaces the join target (PathBuf::join), which
    // the containment check below then rejects unless it is inside the root.
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };

    // Lexical normalization: fold `.` components and apply `..` by popping.
    // A `..` that pops past the path's own root is an escape by definition.
    let mut norm = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !norm.pop() {
                    return Err(escape());
                }
            }
            other => norm.push(other.as_os_str()),
        }
    }

    // Containment check #1: lexical (catches traversal and absolute escape).
    if !is_within(root, &norm) {
        return Err(escape());
    }

    // Containment check #2: symlink/junction escape. Canonicalize the
    // deepest *existing* ancestor of `norm`; the non-existent tail is
    // re-joined afterwards (it cannot be a link — it does not exist).
    let mut probe = norm;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::canonicalize(&probe) {
            Ok(canon) => {
                let mut resolved = canon;
                for part in tail.iter().rev() {
                    resolved.push(part);
                }
                if !is_within(root, &resolved) {
                    return Err(format!(
                        "path `{user}` resolves through a link outside the workspace root"
                    ));
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => match probe.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    if !probe.pop() {
                        return Err(format!("cannot resolve path `{user}`: {err}"));
                    }
                }
                None => return Err(format!("cannot resolve path `{user}`: {err}")),
            },
            Err(err) => return Err(format!("cannot resolve path `{user}`: {err}")),
        }
    }
}

/// Case-insensitive-on-Windows component-wise containment
/// (`candidate == root` counts as within). `\\?\` verbatim prefixes are
/// stripped on both sides so a canonical path can be checked against a
/// lexical one.
fn is_within(root: &Path, candidate: &Path) -> bool {
    let root = de_verbatim(root);
    let candidate = de_verbatim(candidate);
    let mut root = root.components();
    let mut candidate = candidate.components();
    loop {
        match (root.next(), candidate.next()) {
            (None, _) => return true, // candidate is at or below the root
            (Some(_), None) => return false,
            (Some(r), Some(c)) => {
                let r = r.as_os_str().to_string_lossy();
                let c = c.as_os_str().to_string_lossy();
                #[cfg(windows)]
                let equal = r.eq_ignore_ascii_case(&c);
                #[cfg(not(windows))]
                let equal = r == c;
                if !equal {
                    return false;
                }
            }
        }
    }
}

/// Case-insensitive-on-Windows path equality (verbatim prefixes
/// stripped) — the "is this the workspace root itself?" refusal check.
fn same_path(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

/// Strip Windows' `\\?\` verbatim prefix (canonicalize adds it) so a path
/// can be shown to a user, compared lexically, or handed to a child
/// process that may not understand verbatim paths.
pub(crate) fn de_verbatim(path: &Path) -> PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Workspace-relative display form of an already-rooted path
/// (forward slashes, verbatim prefix stripped).
pub(crate) fn rel_display(root: &Path, path: &Path) -> String {
    let plain = de_verbatim(path);
    let rel = plain
        .strip_prefix(de_verbatim(root))
        .unwrap_or(&plain)
        .to_string_lossy()
        .replace('\\', "/");
    if rel.is_empty() {
        ".".to_owned()
    } else {
        rel
    }
}

/// Truncate `text` to `cap` **bytes** on a UTF-8 char boundary, appending
/// the §17.2 truncation notice. Returns `(text, truncated)`.
pub(crate) fn cap_output(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_owned(), false);
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}\n[output truncated at {}KB]", &text[..end], cap / 1024),
        true,
    )
}

// -----------------------------------------------------------------------------
// Read tools (§17.1 — "none (read-only)")
// -----------------------------------------------------------------------------

/// `read_file(path, start_line?, end_line?)` — runtime fs read, path
/// resolved against the workspace root (§17.1). `start_line` /
/// `end_line` are 1-based inclusive. A full read is capped at
/// [`MAX_READ_BYTES`]; a line-slice request may read a larger file because
/// only the requested window is streamed (and capped) — so the cap error's
/// "read a slice" advice is actually actionable.
pub(crate) fn read_file(
    root: &Path,
    id: &str,
    path: &str,
    start_line: Option<u64>,
    end_line: Option<u64>,
) -> ToolResult {
    let resolved = match resolve_rooted(root, path) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(id, e),
    };
    let rel = rel_display(root, &resolved);
    if let Err(message) = reject_secret(root, &resolved) {
        return ToolResult::error(id, message);
    }

    let meta = match std::fs::metadata(&resolved) {
        Ok(m) => m,
        Err(e) => return ToolResult::error(id, format!("cannot read `{rel}`: {e}")),
    };
    if meta.is_dir() {
        return ToolResult::error(id, format!("`{rel}` is a directory (use list_dir)"));
    }
    // §17.2 guard: bounded reads. The cap applies to a full read; a slice
    // request on an oversize file is streamed instead of read whole.
    if meta.len() > MAX_READ_BYTES {
        if start_line.is_none() && end_line.is_none() {
            return ToolResult::error(
                id,
                format!(
                    "`{rel}` is {} bytes — over the 4MB read cap; read a slice with start_line/end_line instead",
                    meta.len()
                ),
            );
        }
        return read_line_slice(id, &rel, &resolved, meta.len(), start_line, end_line);
    }
    let bytes = match std::fs::read(&resolved) {
        Ok(b) => b,
        Err(e) => return ToolResult::error(id, format!("cannot read `{rel}`: {e}")),
    };
    let full = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<&str> = full.lines().collect();
    let total = lines.len() as u64;

    let start = start_line.unwrap_or(1).max(1);
    if start > total {
        return ToolResult::error(
            id,
            format!("`{rel}` has {total} lines — start_line {start} is past the end"),
        );
    }
    let end = end_line.map_or(total, |e| e.min(total));
    if end < start {
        return ToolResult::error(
            id,
            format!("invalid line range: end_line {end} < start_line {start}"),
        );
    }

    let slice: Vec<&str> = lines[(start - 1) as usize..end as usize].to_vec();
    ToolResult {
        id: id.to_owned(),
        ok: true,
        content: format!(
            "{} (lines {}-{} of {}, {} bytes)\n{}",
            rel,
            start,
            end,
            total,
            bytes.len(),
            slice.join("\n")
        ),
    }
}

/// §17.2 bounded read of a file larger than [`MAX_READ_BYTES`]: stream it
/// line by line and keep only the `[start_line, end_line]` window. The
/// window text is capped at [`MAX_READ_BYTES`] (a single monster line is cut
/// on a char boundary with a visible notice), and the file is never held in
/// memory whole. The total line count still needs a full pass, but only
/// counters — not content — grow with the file.
fn read_line_slice(
    id: &str,
    rel: &str,
    resolved: &Path,
    file_len: u64,
    start_line: Option<u64>,
    end_line: Option<u64>,
) -> ToolResult {
    let file = match std::fs::File::open(resolved) {
        Ok(f) => f,
        Err(e) => return ToolResult::error(id, format!("cannot read `{rel}`: {e}")),
    };
    let mut reader = std::io::BufReader::new(file);
    let start = start_line.unwrap_or(1).max(1);
    let mut total: u64 = 0;
    let mut kept = String::new();
    let mut truncated = false;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        buf.clear();
        let n = match std::io::BufRead::read_until(&mut reader, b'\n', &mut buf) {
            Ok(n) => n,
            Err(e) => return ToolResult::error(id, format!("cannot read `{rel}`: {e}")),
        };
        if n == 0 {
            break;
        }
        total += 1;
        if total < start || end_line.is_some_and(|end| total > end) || truncated {
            continue;
        }
        // `str::lines()` semantics: strip the line ending (`\n` or `\r\n`).
        let mut line: &[u8] = &buf;
        if let Some(rest) = line.strip_suffix(b"\n") {
            line = rest;
        }
        if let Some(rest) = line.strip_suffix(b"\r") {
            line = rest;
        }
        let text = String::from_utf8_lossy(line);
        let room = (MAX_READ_BYTES as usize).saturating_sub(kept.len());
        let sep = usize::from(!kept.is_empty());
        if sep + text.len() > room {
            if room > sep {
                if sep == 1 {
                    kept.push('\n');
                }
                let mut cut = room - sep;
                while cut > 0 && !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                kept.push_str(&text[..cut]);
            }
            truncated = true;
            continue;
        }
        if sep == 1 {
            kept.push('\n');
        }
        kept.push_str(&text);
    }

    if start > total {
        return ToolResult::error(
            id,
            format!("`{rel}` has {total} lines — start_line {start} is past the end"),
        );
    }
    let end = end_line.map_or(total, |e| e.min(total));
    if end < start {
        return ToolResult::error(
            id,
            format!("invalid line range: end_line {end} < start_line {start}"),
        );
    }
    let notice = if truncated {
        format!("\n[slice truncated at {}KB]", MAX_READ_BYTES / 1024)
    } else {
        String::new()
    };
    ToolResult {
        id: id.to_owned(),
        ok: true,
        content: format!(
            "{} (lines {}-{} of {}, {} bytes)\n{}{}",
            rel, start, end, total, file_len, kept, notice
        ),
    }
}

/// `list_dir(path)` — the `selora-fs` tree node for one directory (§17.1),
/// rendered one level deep. Entries hidden or `.gitignore`d are skipped
/// (the same walk the file-tree sidebar uses).
pub(crate) fn list_dir(root: &Path, id: &str, path: &str) -> ToolResult {
    let resolved = match resolve_rooted(root, path) {
        Ok(p) => p,
        Err(e) => return ToolResult::error(id, e),
    };
    let rel = rel_display(root, &resolved);
    if let Err(message) = reject_secret(root, &resolved) {
        return ToolResult::error(id, message);
    }
    if !resolved.is_dir() {
        return ToolResult::error(id, format!("`{rel}` is not a directory"));
    }
    // §17.1: the implementation is the selora-fs tree node — gitignore and
    // hidden-entry semantics come with it.
    let node = skarn_fs::tree::build_tree(&resolved);
    let mut out = String::new();
    for child in &node.children {
        let child_rel = if child.path == resolved {
            child.name.clone()
        } else {
            rel_display(root, &child.path)
        };
        if is_secret_path(&child_rel) {
            continue;
        }
        out.push_str(&child_rel);
        if child.is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    if node.truncated {
        out.push_str(&format!(
            "… (more than {} entries, truncated)\n",
            skarn_fs::tree::MAX_CHILDREN
        ));
    }
    let (capped, _) = cap_output(&out, MAX_OUTPUT_BYTES);
    ToolResult {
        id: id.to_owned(),
        ok: true,
        content: if capped.is_empty() {
            format!("`{rel}` is empty\n")
        } else {
            capped
        },
    }
}

/// `fuzzy_find(query)` — `skarn_fs::fuzzy_find` over the workspace's
/// files (§17.1), rendered as workspace-relative paths.
pub(crate) fn fuzzy_find(root: &Path, id: &str, query: &str) -> ToolResult {
    if query.trim().is_empty() {
        return ToolResult::error(id, "query must not be empty");
    }
    let tree = skarn_fs::tree::build_tree(root);
    let mut items: Vec<(PathBuf, String)> = Vec::new();
    collect_files(&tree, root, &mut items);
    items.retain(|(path, _)| !is_secret_path(&rel_display(root, path)));
    let matches = skarn_fs::fuzzy::fuzzy_find(query, &items);
    if matches.is_empty() {
        // No match is a (successful) empty answer, not a tool error.
        return ToolResult {
            id: id.to_owned(),
            ok: true,
            content: format!("no files match `{query}`"),
        };
    }
    let mut out = String::new();
    for m in &matches {
        out.push_str(&rel_display(root, &m.path));
        out.push('\n');
    }
    let (capped, _) = cap_output(&out, MAX_OUTPUT_BYTES);
    ToolResult {
        id: id.to_owned(),
        ok: true,
        content: format!("{} match(es)\n{}", matches.len(), capped),
    }
}

/// Collect the tree's files as fuzzy candidates: `(path, parent tag)`
/// (the tag is the Ctrl+P palette's right column — the parent directory).
fn collect_files(
    node: &skarn_fs::tree::FileTreeNode,
    root: &Path,
    out: &mut Vec<(PathBuf, String)>,
) {
    for child in &node.children {
        if child.is_dir {
            collect_files(child, root, out);
        } else {
            let tag = child
                .path
                .parent()
                .map(|p| rel_display(root, p))
                .unwrap_or_default();
            out.push((child.path.clone(), tag));
        }
    }
}

/// `search_files(query)` — `skarn_fs::search_in_files` (ripgrep when on
/// PATH, else the built-in scanner) over the workspace (§17.1), rendered
/// as `path:line: text` hits.
pub(crate) fn search_files(root: &Path, id: &str, query: &str) -> ToolResult {
    if query.is_empty() {
        return ToolResult::error(id, "query must not be empty");
    }
    let q = skarn_fs::search::SearchQuery::new(query);
    let hits = match skarn_fs::search::search_in_files(&de_verbatim(root), &q) {
        Ok(h) => h,
        Err(e) => return ToolResult::error(id, format!("search failed: {e}")),
    };
    if hits.is_empty() {
        // No hits is a (successful) empty answer, not a tool error.
        return ToolResult {
            id: id.to_owned(),
            ok: true,
            content: format!("no matches for `{query}`"),
        };
    }
    let hits: Vec<_> = hits
        .into_iter()
        .filter(|hit| {
            let path = std::fs::canonicalize(&hit.path).unwrap_or_else(|_| hit.path.clone());
            !is_secret_path(&rel_display(root, &path))
                && !is_secret_path(&rel_display(root, &hit.path))
        })
        .collect();
    let total = hits.len();
    let mut out = String::new();
    for hit in hits.iter().take(MAX_SEARCH_HITS) {
        out.push_str(&format!(
            "{}:{}: {}\n",
            rel_display(root, &hit.path),
            hit.line_number,
            hit.line_text.trim_end()
        ));
    }
    if total > MAX_SEARCH_HITS {
        out.push_str(&format!(
            "… (showing the first {MAX_SEARCH_HITS} of {total} matches)\n"
        ));
    }
    let (capped, _) = cap_output(&out, MAX_OUTPUT_BYTES);
    ToolResult {
        id: id.to_owned(),
        ok: true,
        content: format!("{total} match(es)\n{capped}"),
    }
}

// -----------------------------------------------------------------------------
// Write tools (§17.1 / §17.4 — planned, then applied on approval)
// -----------------------------------------------------------------------------

/// Plan a `write_file(path, content)`: resolve the path (§17.2), refuse
/// oversize content and directories, read the current bytes for the diff,
/// and build the unified diff (create = all additions). The router turns
/// the `(rel_path, diff)` into `ApprovalCard::Diff`; nothing is written.
pub(crate) fn plan_write(
    root: &Path,
    path: &str,
    content: &str,
) -> Result<(String, String), String> {
    let resolved = resolve_rooted(root, path)?;
    reject_secret(root, &resolved)?;
    if same_path(&resolved, root) {
        return Err("refusing to operate on the workspace root itself".to_owned());
    }
    if resolved.is_dir() {
        return Err(format!(
            "`{}` is a directory; write_file writes files",
            rel_display(root, &resolved)
        ));
    }
    // §17.2 bounded I/O (write side of the cap): an approved write may not
    // exceed 4MB.
    if content.len() as u64 > MAX_READ_BYTES {
        return Err(format!(
            "content is {} bytes — over the 4MB write cap",
            content.len()
        ));
    }
    let rel = rel_display(root, &resolved);
    let old = match std::fs::metadata(&resolved) {
        Ok(meta) if meta.is_file() => {
            if meta.len() > MAX_READ_BYTES {
                return Err(format!(
                    "`{}` is {} bytes — over the 4MB cap, cannot build a diff for it",
                    rel_display(root, &resolved),
                    meta.len()
                ));
            }
            String::from_utf8_lossy(&std::fs::read(&resolved).map_err(|e| e.to_string())?)
                .into_owned()
        }
        _ => String::new(), // create (or a dangling symlink, which the write replaces)
    };
    // An empty `content` is a full-file delete, not a write. Refuse it when
    // the target holds bytes: the agent's own `delete_file` (and its card) is
    // the explicit path for that, so a malformed/empty tool argument can
    // never silently truncate a file the user is looking at.
    if content.is_empty() && !old.is_empty() {
        return Err(format!(
            "refusing to replace non-empty `{rel}` with empty content — pass the file's full new content, or use `delete_file` to delete it"
        ));
    }
    let diff = unified_diff(&rel, &old, content);
    Ok((rel, diff))
}

/// Plan an `edit_file(path, old_str, new_str, replace_all?)`: resolve the
/// path (§17.2), read the current bytes, and validate the match
/// (§17.1 "exact-match string replace"): not found, ambiguous without
/// `replace_all`, and no-op are all model-visible errors. Returns the
/// `(rel_path, diff)` for the approval card; nothing is written.
pub(crate) fn plan_edit(
    root: &Path,
    path: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
) -> Result<(String, String), String> {
    let resolved = resolve_rooted(root, path)?;
    reject_secret(root, &resolved)?;
    if same_path(&resolved, root) {
        return Err("refusing to operate on the workspace root itself".to_owned());
    }
    if !resolved.is_file() {
        return Err(format!(
            "`{}` is not an existing file",
            rel_display(root, &resolved)
        ));
    }
    if old_str.is_empty() {
        return Err("old_str must not be empty".to_owned());
    }
    if old_str == new_str {
        return Err("old_str and new_str are identical — the edit would change nothing".to_owned());
    }
    let meta = std::fs::metadata(&resolved).map_err(|e| e.to_string())?;
    if meta.len() > MAX_READ_BYTES {
        return Err(format!(
            "`{}` is {} bytes — over the 4MB cap",
            rel_display(root, &resolved),
            meta.len()
        ));
    }
    let old =
        String::from_utf8_lossy(&std::fs::read(&resolved).map_err(|e| e.to_string())?).into_owned();

    let occurrences = old.matches(old_str).count();
    match occurrences {
        0 => {
            return Err(format!(
                "old_str not found in `{}` — copy it exactly from the file (read_file first)",
                rel_display(root, &resolved)
            ))
        }
        n if n > 1 && !replace_all => {
            return Err(format!(
                "old_str occurs {n} times in `{}` — add surrounding context to make it unique, or pass replace_all: true",
                rel_display(root, &resolved)
            ))
        }
        _ => {}
    }

    let new_content = if replace_all {
        old.replace(old_str, new_str)
    } else {
        old.replacen(old_str, new_str, 1)
    };
    let rel = rel_display(root, &resolved);
    let diff = unified_diff(&rel, &old, &new_content);
    Ok((rel, diff))
}

/// Apply an **approved** `write_file` (§17.4): delegates to [`apply_write_cas`].
#[cfg(test)]
pub(crate) fn apply_write(root: &Path, id: &str, path: &str, content: &str) -> ToolResult {
    apply_write_cas(root, id, path, content, None, true).tool_result
}

/// CAS-protected `write_file` apply: validates `expected` (when supplied),
/// writes atomically via `.skarn-tmp-*` sibling + rename, and returns before/after
/// bytes for `TurnCheckpointStore`.
pub(crate) fn apply_write_cas(
    root: &Path,
    id: &str,
    path: &str,
    content: &str,
    expected: Option<&ExpectedFilePrecondition>,
    write_backup: bool,
) -> CasMutationResult {
    if let Err(e) = plan_write(root, path, content) {
        return CasMutationResult {
            tool_result: ToolResult::error(id, e),
            file_change: None,
        };
    }
    let resolved = match resolve_rooted(root, path) {
        Ok(p) => p,
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    let rel = rel_display(root, &resolved);
    let before_bytes = match verify_cas_precondition(&resolved, &rel, expected) {
        Ok(bytes) => bytes,
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    match write_atomic_with_backup(&resolved, content, write_backup) {
        Ok(backup) => CasMutationResult {
            tool_result: ToolResult {
                id: id.to_owned(),
                ok: true,
                content: match backup {
                    Some(backup) => format!(
                        "wrote {} bytes to `{rel}` (previous content saved to `{}`)",
                        content.len(),
                        rel_display(root, &backup)
                    ),
                    None => format!("wrote {} bytes to `{rel}`", content.len()),
                },
            },
            file_change: Some(skarn_decision::TurnFileChange {
                rel_path: rel,
                before_bytes,
                after_bytes: Some(content.as_bytes().to_vec()),
            }),
        },
        Err(e) => CasMutationResult {
            tool_result: ToolResult::error(id, format!("write to `{rel}` failed: {e}")),
            file_change: None,
        },
    }
}

/// Apply an **approved** `edit_file` (§17.4): delegates to [`apply_edit_cas`].
#[cfg(test)]
pub(crate) fn apply_edit(
    root: &Path,
    id: &str,
    path: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
) -> ToolResult {
    apply_edit_cas(root, id, path, old_str, new_str, replace_all, None, true).tool_result
}

/// CAS-protected `edit_file` apply: validates `expected` SHA-256 (when supplied)
/// and `old_str` uniqueness, writes atomically via `.skarn-tmp-*` sibling + rename,
/// and returns before/after bytes for `TurnCheckpointStore`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_edit_cas(
    root: &Path,
    id: &str,
    path: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
    expected: Option<&ExpectedFilePrecondition>,
    write_backup: bool,
) -> CasMutationResult {
    let resolved = match resolve_rooted(root, path) {
        Ok(p) => p,
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    let rel = rel_display(root, &resolved);
    if !resolved.is_file() {
        return CasMutationResult {
            tool_result: ToolResult::error(id, format!("`{rel}` is not an existing file")),
            file_change: None,
        };
    }
    let before_bytes = match verify_cas_precondition(&resolved, &rel, expected) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, format!("`{rel}` does not exist")),
                file_change: None,
            }
        }
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    let old = String::from_utf8_lossy(&before_bytes).into_owned();
    let occurrences = old.matches(old_str).count();
    if occurrences == 0 {
        return CasMutationResult {
            tool_result: ToolResult::error(
                id,
                format!(
                    "`{rel}` changed since the edit was approved — old_str no longer matches; re-request the edit"
                ),
            ),
            file_change: None,
        };
    }
    if occurrences > 1 && !replace_all {
        return CasMutationResult {
            tool_result: ToolResult::error(
                id,
                format!(
                    "`{rel}` changed since the edit was approved — old_str is now ambiguous; re-request the edit"
                ),
            ),
            file_change: None,
        };
    }
    let new_content = if replace_all {
        old.replace(old_str, new_str)
    } else {
        old.replacen(old_str, new_str, 1)
    };
    match write_atomic_with_backup(&resolved, &new_content, write_backup) {
        Ok(backup) => CasMutationResult {
            tool_result: ToolResult {
                id: id.to_owned(),
                ok: true,
                content: match backup {
                    Some(backup) => format!(
                        "edited `{rel}` ({occurrences} replacement(s); previous content saved to `{}`)",
                        rel_display(root, &backup)
                    ),
                    None => format!("edited `{rel}` ({occurrences} replacement(s))"),
                },
            },
            file_change: Some(skarn_decision::TurnFileChange {
                rel_path: rel,
                before_bytes: Some(before_bytes),
                after_bytes: Some(new_content.as_bytes().to_vec()),
            }),
        },
        Err(e) => CasMutationResult {
            tool_result: ToolResult::error(id, format!("edit of `{rel}` failed: {e}")),
            file_change: None,
        },
    }
}

/// Apply an **approved** `delete_file` (§17.4): delegates to [`apply_delete_cas`].
#[cfg(test)]
pub(crate) fn apply_delete(root: &Path, id: &str, path: &str) -> ToolResult {
    apply_delete_cas(root, id, path, None).tool_result
}

/// CAS-protected `delete_file` apply: validates `expected` SHA-256 (when supplied),
/// removes the file, and returns before/after bytes for `TurnCheckpointStore`.
pub(crate) fn apply_delete_cas(
    root: &Path,
    id: &str,
    path: &str,
    expected: Option<&ExpectedFilePrecondition>,
) -> CasMutationResult {
    let resolved = match resolve_rooted(root, path) {
        Ok(p) => p,
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    let rel = rel_display(root, &resolved);
    if same_path(&resolved, root) {
        return CasMutationResult {
            tool_result: ToolResult::error(id, "refusing to delete the workspace root".to_owned()),
            file_change: None,
        };
    }
    match std::fs::symlink_metadata(&resolved) {
        Ok(meta) if meta.is_dir() => {
            return CasMutationResult {
                tool_result: ToolResult::error(
                    id,
                    format!("`{rel}` is a directory — delete_file deletes files only"),
                ),
                file_change: None,
            }
        }
        Ok(_) => {}
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, format!("cannot delete `{rel}`: {e}")),
                file_change: None,
            }
        }
    }
    let before_bytes = match verify_cas_precondition(&resolved, &rel, expected) {
        Ok(bytes) => bytes,
        Err(e) => {
            return CasMutationResult {
                tool_result: ToolResult::error(id, e),
                file_change: None,
            }
        }
    };
    match std::fs::remove_file(&resolved) {
        Ok(()) => CasMutationResult {
            tool_result: ToolResult {
                id: id.to_owned(),
                ok: true,
                content: format!("deleted `{rel}`"),
            },
            file_change: Some(skarn_decision::TurnFileChange {
                rel_path: rel,
                before_bytes,
                after_bytes: None,
            }),
        },
        Err(e) => CasMutationResult {
            tool_result: ToolResult::error(id, format!("cannot delete `{rel}`: {e}")),
            file_change: None,
        },
    }
}

/// A unique sibling temp path for `target`:
/// `.<name>.skarn-tmp-<pid>-<seq>`. The process id plus a per-process
/// atomic counter means two concurrent approved writes to the same target
/// never share a temp file (the old deterministic sidecar name made them
/// truncate each other's temp).
fn temp_sibling(target: &Path) -> PathBuf {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let seq = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.skarn-tmp-{}-{seq}", std::process::id()))
}

/// The one-slot recovery sidecar for `target`: `.<name>.skarn-bak`.
/// Replaced (atomically) on every write, so the file the user was looking at
/// is always available after an approved overwrite.
fn backup_sibling(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.skarn-bak"))
}

/// Copy the current bytes of `target` to [`backup_sibling`], atomically.
/// Returns `None` when the target does not exist yet (a create has nothing
/// to preserve) or is not a regular file. A backup failure aborts the write:
/// losing the recovery copy is worse than failing the edit.
fn backup_existing(target: &Path) -> std::io::Result<Option<PathBuf>> {
    match std::fs::metadata(target) {
        Ok(meta) if meta.is_file() => {
            let backup = backup_sibling(target);
            let tmp = temp_sibling(&backup);
            if let Err(e) = std::fs::copy(target, &tmp) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            match std::fs::rename(&tmp, &backup) {
                Ok(()) => Ok(Some(backup)),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    Err(e)
                }
            }
        }
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Atomic-ish write: a unique temp file in the target directory, then
/// rename over the target (std's rename replaces an existing file on
/// Windows). Parent directories are created (write_file into a new folder is
/// a create). The temp is created with `create_new` so even a cross-process
/// name clash is an error rather than silently sharing a temp file, and it
/// is removed if writing or renaming fails.
///
/// Before replacing an existing regular file, its current bytes are copied
/// to [`backup_sibling`] (also via temp + rename). Returns the backup path
/// when one was written.
#[cfg(test)]
fn write_atomic(target: &Path, content: &str) -> std::io::Result<Option<PathBuf>> {
    write_atomic_with_backup(target, content, true)
}

fn write_atomic_with_backup(
    target: &Path,
    content: &str,
    create_backup: bool,
) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let backup = if create_backup {
        backup_existing(target)?
    } else {
        None
    };
    let tmp = temp_sibling(target);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    if let Err(e) = std::io::Write::write_all(&mut file, content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // Windows cannot rename over the target while our temp handle is open.
    drop(file);
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(backup),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// -----------------------------------------------------------------------------
// Unified diff (the §17.1/§17.4 diff card body)
// -----------------------------------------------------------------------------

/// Build a unified diff (headers, `@@` hunks, 3 context lines) between
/// `old` and `new`. `old` empty → a create, shown as all additions
/// (§17.1) with `--- /dev/null`.
pub(crate) fn unified_diff(rel: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = patience_ops(&old_lines, &new_lines);

    // Tag every line with its diff role and its old/new line number.
    #[derive(Clone, Copy)]
    struct Tagged {
        tag: u8, // b' ', b'-', b'+'
        old_no: usize,
        new_no: usize,
    }
    let mut tagged: Vec<Tagged> = Vec::new();
    let (mut o, mut n) = (0usize, 0usize);
    for op in &ops {
        match op {
            Op::Keep(_) => {
                tagged.push(Tagged {
                    tag: b' ',
                    old_no: o + 1,
                    new_no: n + 1,
                });
                o += 1;
                n += 1;
            }
            Op::Del(_) => {
                tagged.push(Tagged {
                    tag: b'-',
                    old_no: o + 1,
                    new_no: n + 1,
                });
                o += 1;
            }
            Op::Add(_) => {
                tagged.push(Tagged {
                    tag: b'+',
                    old_no: o + 1,
                    new_no: n + 1,
                });
                n += 1;
            }
        }
    }

    let mut out = String::new();
    if old.is_empty() {
        out.push_str("--- /dev/null\n");
    } else {
        out.push_str(&format!("--- a/{rel}\n"));
    }
    out.push_str(&format!("+++ b/{rel}\n"));

    // A tagged index is in a hunk when a change is within DIFF_CONTEXT of
    // it; consecutive in-hunk runs form hunks (changes separated by more
    // than 2*DIFF_CONTEXT context lines split).
    let changed = |i: usize| tagged[i].tag != b' ';
    let in_hunk = |i: usize| {
        let lo = i.saturating_sub(DIFF_CONTEXT);
        let hi = (i + DIFF_CONTEXT + 1).min(tagged.len());
        (lo..hi).any(changed)
    };

    let mut i = 0;
    let mut lines_emitted = 0usize;
    while i < tagged.len() {
        if !in_hunk(i) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < tagged.len() && in_hunk(end) {
            end += 1;
        }
        i = end;

        // Count and start lines for the @@ header.
        let old_count = tagged[start..end].iter().filter(|t| t.tag != b'+').count();
        let new_count = tagged[start..end].iter().filter(|t| t.tag != b'-').count();
        // 1-based start; an empty side anchors to the line before the hunk
        // (git's `@@ -x,0 +y,n @@` convention).
        let old_start = if old_count == 0 {
            tagged[start].old_no.saturating_sub(1)
        } else {
            tagged[start..end]
                .iter()
                .find(|t| t.tag != b'+')
                .map(|t| t.old_no)
                .unwrap_or(0)
        };
        let new_start = if new_count == 0 {
            tagged[start].new_no.saturating_sub(1)
        } else {
            tagged[start..end]
                .iter()
                .find(|t| t.tag != b'-')
                .map(|t| t.new_no)
                .unwrap_or(0)
        };
        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for t in &tagged[start..end] {
            let text = match t.tag {
                b'-' => old_lines[t.old_no - 1],
                b'+' => new_lines[t.new_no - 1],
                _ => {
                    if t.old_no >= 1 && t.old_no <= old_lines.len() {
                        old_lines[t.old_no - 1]
                    } else {
                        new_lines[t.new_no - 1]
                    }
                }
            };
            out.push(t.tag as char);
            out.push_str(text);
            out.push('\n');
            lines_emitted += 1;
            if lines_emitted >= MAX_DIFF_LINES {
                out.push_str(&format!("… (diff truncated at {MAX_DIFF_LINES} lines)\n"));
                return out;
            }
        }
    }
    out
}

/// One patience-diff step over line indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep(usize), // index into old
    Del(usize),  // index into old
    Add(usize),  // index into new
}

/// Patience diff (unique-common anchors + LIS, prefix/suffix trim).
/// O(n log n) time, O(n) space — no quadratic blow-up on large files, and
/// it produces the "right" hunks for code edits (matching unique lines
/// like signatures or braces).
fn patience_ops(old: &[&str], new: &[&str]) -> Vec<Op> {
    let mut ops = Vec::new();
    let span = Span::new(old.len(), new.len());
    diff_range(old, new, span, &mut ops, 0);
    ops
}

/// Half-open index ranges `[os, oe)` into `old` and `[ns, ne)` into `new`
/// that one `diff_range` step covers.
#[derive(Clone, Copy)]
struct Span {
    os: usize,
    oe: usize,
    ns: usize,
    ne: usize,
}

impl Span {
    fn new(old_len: usize, new_len: usize) -> Self {
        Self {
            os: 0,
            oe: old_len,
            ns: 0,
            ne: new_len,
        }
    }

    fn from(po: usize, oe: usize, pn: usize, ne: usize) -> Self {
        Self {
            os: po,
            oe,
            ns: pn,
            ne,
        }
    }
}

fn diff_range(old: &[&str], new: &[&str], span: Span, ops: &mut Vec<Op>, depth: usize) {
    let Span {
        mut os,
        mut oe,
        mut ns,
        mut ne,
    } = span;

    // Hard bound on recursion (anchors strictly split the ranges, so this
    // should never trigger — kept as a safety net regardless).
    if depth > 64 {
        ops.extend((os..oe).map(Op::Del));
        ops.extend((ns..ne).map(Op::Add));
        return;
    }
    if os >= oe && ns >= ne {
        return;
    }

    // Trim the common head.
    while os < oe && ns < ne && old[os] == new[ns] {
        ops.push(Op::Keep(os));
        os += 1;
        ns += 1;
    }
    // Trim the common tail (reversed, appended after the recursion).
    let mut tail: Vec<Op> = Vec::new();
    while oe > os && ne > ns && old[oe - 1] == new[ne - 1] {
        oe -= 1;
        ne -= 1;
        tail.push(Op::Keep(oe));
    }

    if os == oe {
        ops.extend((ns..ne).map(Op::Add));
    } else if ns == ne {
        ops.extend((os..oe).map(Op::Del));
    } else {
        // Anchors: lines that are unique on both sides and equal.
        let mut old_count: HashMap<&str, u32> = HashMap::new();
        let mut new_count: HashMap<&str, u32> = HashMap::new();
        for line in &old[os..oe] {
            *old_count.entry(line).or_insert(0) += 1;
        }
        for line in &new[ns..ne] {
            *new_count.entry(line).or_insert(0) += 1;
        }
        let mut pairs: Vec<(usize, usize)> = Vec::new(); // (old_idx, new_idx)
        for (i, line) in old.iter().enumerate().take(oe).skip(os) {
            if old_count.get(line) == Some(&1) && new_count.get(line) == Some(&1) {
                // unique on both sides — the matching new index exists
                let j = new[ns..ne]
                    .iter()
                    .position(|l| *l == *line)
                    .expect("unique match")
                    + ns;
                pairs.push((i, j));
            }
        }
        // LIS over new indices (pairs are already ordered by old index).
        let anchors = longest_increasing(&pairs);
        if anchors.is_empty() {
            ops.extend((os..oe).map(Op::Del));
            ops.extend((ns..ne).map(Op::Add));
        } else {
            let (mut po, mut pn) = (os, ns);
            for (i, j) in anchors {
                diff_range(old, new, Span::from(po, i, pn, j), ops, depth + 1);
                ops.push(Op::Keep(i));
                po = i + 1;
                pn = j + 1;
            }
            diff_range(old, new, Span::from(po, oe, pn, ne), ops, depth + 1);
        }
    }
    ops.extend(tail.into_iter().rev());
}

/// Longest strictly-increasing subsequence over the pairs' new indices;
/// returns the chosen pairs in order. O(k log k).
fn longest_increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // `tails[k]` = index into `pairs` of the smallest-ended chain of length k+1
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; pairs.len()];
    for (idx, &(_, n)) in pairs.iter().enumerate() {
        // binary search for the first tail whose new-index is >= n
        let mut lo = 0;
        let mut hi = tails.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if pairs[tails[mid]].1 < n {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 {
            prev[idx] = Some(tails[lo - 1]);
        }
        if lo == tails.len() {
            tails.push(idx);
        } else {
            tails[lo] = idx;
        }
    }
    let mut result = Vec::new();
    let mut cur = tails.last().copied();
    while let Some(idx) = cur {
        result.push(pairs[idx]);
        cur = prev[idx];
    }
    result.reverse();
    result
}

// -----------------------------------------------------------------------------
// Tests — every guard gets its own test, citing its §
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::testutil::{temp_root, touch};
    use super::*;

    fn read(root: &Path, path: &str) -> ToolResult {
        read_file(root, "call_1", path, None, None)
    }

    #[test]
    fn secret_paths_are_blocked_from_reads_and_diff_planning() {
        let root = temp_root("secret-guard");
        touch(&root, "Credentials/config.txt", "private-value\n");
        touch(&root, "public.txt", "public-value\n");
        assert!(is_secret_path("folder\\.ENV.production"));
        assert!(is_secret_path("Credentials/config.txt"));
        assert!(!read(&root, "Credentials/config.txt").ok);
        assert!(!list_dir(&root, "id", "Credentials").ok);
        assert!(plan_write(&root, "Credentials/config.txt", "new").is_err());
        assert!(plan_edit(&root, "Credentials/config.txt", "private", "new", false).is_err());
        assert!(read(&root, "public.txt").ok);
        assert!(!list_dir(&root, "id", ".").content.contains("Credentials"));
        assert!(!fuzzy_find(&root, "id", "config")
            .content
            .contains("Credentials"));
        assert!(!search_files(&root, "id", "private-value")
            .content
            .contains("private-value\n"));
    }

    // -- §17.2 path rooting ----------------------------------------------------

    #[test]
    fn s17_2_relative_path_resolves_inside_root() {
        let root = temp_root("path-ok");
        touch(&root, "src/main.rs", "fn main() {}\n");
        let resolved = resolve_rooted(&root, "src/main.rs").expect("inside root");
        assert!(resolved.is_file());
        assert!(is_within(&root, &resolved));
    }

    #[test]
    fn s17_2_dot_resolves_to_the_root_itself() {
        let root = temp_root("path-dot");
        let resolved = resolve_rooted(&root, ".").expect("root");
        assert!(is_within(&root, &resolved));
    }

    #[test]
    fn s17_2_absolute_path_inside_root_is_accepted() {
        let root = temp_root("path-abs-in");
        touch(&root, "a.txt", "x");
        let abs = root.join("a.txt");
        let resolved = resolve_rooted(&root, &abs.to_string_lossy()).expect("inside");
        assert!(resolved.is_file());
    }

    #[test]
    fn s17_2_dotdot_traversal_is_rejected() {
        let root = temp_root("path-escape");
        std::fs::write(root.join("../secret.txt"), "x").expect("fixture outside root");
        for bad in [
            "../secret.txt",
            "a/../../secret.txt",
            "..\\secret.txt",
            "sub/../../../secret.txt",
        ] {
            let err = resolve_rooted(&root, bad).expect_err(bad);
            assert!(
                err.contains("escapes"),
                "`{bad}` should be an escape, got: {err}"
            );
        }
        std::fs::remove_file(root.join("../secret.txt")).ok();
    }

    #[test]
    fn s17_2_absolute_path_escape_is_rejected() {
        let root = temp_root("path-abs-out");
        for bad in [
            "C:\\Windows\\System32\\config.sys",
            "\\\\localhost\\c$\\secret",
            concat!(env!("CARGO_MANIFEST_DIR"), "\\Cargo.toml"),
        ] {
            let err = resolve_rooted(&root, bad).expect_err(bad);
            assert!(
                err.contains("escapes"),
                "`{bad}` should be an escape, got: {err}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn s17_2_junction_escape_is_rejected() {
        let root = temp_root("path-junction");
        let outside = temp_root("path-junction-outside");
        touch(&outside, "secret.txt", "secrets");
        // A junction (directory link) inside the root pointing outside.
        // Junctions need no admin rights, unlike symlinks.
        let link = root.join("link");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("spawn cmd");
        if !status.status.success() {
            eprintln!("skipping: cannot create junctions on this host");
            return;
        }
        let err = resolve_rooted(&root, "link/secret.txt").expect_err("junction escape");
        assert!(
            err.contains("link") || err.contains("escapes"),
            "junction escape should be rejected, got: {err}"
        );
        // A path through the junction to a NEW file is also refused (the
        // canonical ancestor check).
        let err = resolve_rooted(&root, "link/new.txt").expect_err("junction new file");
        assert!(err.contains("outside the workspace root"), "got: {err}");
    }

    #[test]
    #[cfg(windows)]
    fn s17_2_symlink_escape_is_rejected() {
        let root = temp_root("path-symlink");
        let outside = temp_root("path-symlink-outside");
        touch(&outside, "secret.txt", "secrets");
        let link = root.join("escape.rs");
        if std::os::windows::fs::symlink_file(outside.join("secret.txt"), &link).is_err() {
            eprintln!("skipping: symlink creation needs dev-mode/admin on Windows");
            return;
        }
        let err = resolve_rooted(&root, "escape.rs").expect_err("symlink escape");
        assert!(err.contains("outside the workspace root"), "got: {err}");
    }

    #[test]
    fn s17_2_empty_path_is_rejected() {
        let root = temp_root("path-empty");
        assert!(resolve_rooted(&root, "").is_err());
        assert!(resolve_rooted(&root, "   ").is_err());
    }

    // -- §17.2 read cap ---------------------------------------------------------

    #[test]
    fn s17_2_read_over_4mb_is_refused() {
        let root = temp_root("read-cap");
        let big = vec![b'x'; (MAX_READ_BYTES + 1) as usize];
        std::fs::write(root.join("big.bin"), &big).expect("write fixture");
        let result = read(&root, "big.bin");
        assert!(!result.ok);
        assert!(
            result.content.contains("4MB read cap"),
            "cap error expected, got: {}",
            result.content
        );
    }

    #[test]
    fn s17_2_read_exactly_4mb_is_allowed() {
        let root = temp_root("read-edge");
        let exact = vec![b'x'; MAX_READ_BYTES as usize];
        std::fs::write(root.join("exact.txt"), &exact).expect("write fixture");
        let result = read(&root, "exact.txt");
        assert!(result.ok, "{}", result.content);
    }

    #[test]
    fn s17_2_read_slice_of_over_4mb_file_is_allowed() {
        let root = temp_root("read-cap-slice");
        // Three lines, comfortably over the cap: two huge lines then a
        // small one. The slice below only needs the small last line, so
        // it proves the cap is on the returned slice, not the file.
        let mut content = String::new();
        content.push_str(&"a".repeat(3 * 1024 * 1024));
        content.push('\n');
        content.push_str(&"b".repeat(2 * 1024 * 1024));
        content.push('\n');
        content.push_str("needle\n");
        assert!(content.len() as u64 > MAX_READ_BYTES);
        std::fs::write(root.join("big.txt"), &content).expect("write fixture");

        // A full read of the oversize file still fails.
        let full = read(&root, "big.txt");
        assert!(!full.ok);
        assert!(
            full.content.contains("4MB read cap"),
            "cap error expected, got: {}",
            full.content
        );

        // ...but the line slice the error recommends now works.
        let slice = read_file(&root, "call_1", "big.txt", Some(3), Some(3));
        assert!(slice.ok, "slice should be allowed: {}", slice.content);
        assert!(slice.content.contains("needle"));
        assert!(slice.content.contains("lines 3-3 of 3"));
        assert!(!slice.content.contains("aaaaaaaa"));
    }

    // -- read_file --------------------------------------------------------------

    #[test]
    fn read_file_returns_content_with_line_range() {
        let root = temp_root("read-lines");
        touch(&root, "f.txt", "one\ntwo\nthree\nfour\nfive\n");
        let all = read(&root, "f.txt");
        assert!(all.ok);
        assert!(all.content.contains("one\n"));
        assert!(all.content.contains("five"));
        let slice = read_file(&root, "call_1", "f.txt", Some(2), Some(3));
        assert!(slice.ok);
        assert!(slice.content.contains("two\nthree"));
        assert!(!slice.content.contains("one\n"));
        assert!(!slice.content.contains("four"));
        let bad = read_file(&root, "call_1", "f.txt", Some(3), Some(2));
        assert!(!bad.ok);
    }

    #[test]
    fn read_file_missing_is_error_and_dir_is_error() {
        let root = temp_root("read-missing");
        touch(&root, "d/inner.txt", "x");
        let missing = read(&root, "nope.txt");
        assert!(!missing.ok);
        let dir = read(&root, "d");
        assert!(!dir.ok);
        assert!(dir.content.contains("list_dir"));
    }

    // -- list_dir / fuzzy_find / search_files -----------------------------------

    #[test]
    fn list_dir_lists_one_level_skipping_hidden_and_ignored() {
        let root = temp_root("listdir");
        touch(&root, "Cargo.toml", "x");
        touch(&root, "src/main.rs", "x");
        touch(&root, ".hidden", "x");
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").expect("gitignore");
        touch(&root, "ignored.txt", "x");
        let result = list_dir(&root, "call_1", ".");
        assert!(result.ok, "{}", result.content);
        assert!(result.content.contains("src/"));
        assert!(result.content.contains("Cargo.toml"));
        assert!(!result.content.contains(".hidden"));
        assert!(!result.content.contains("ignored.txt"));
        // A subdirectory lists its own level.
        let sub = list_dir(&root, "call_1", "src");
        assert!(sub.ok);
        assert!(sub.content.contains("main.rs"));
        assert!(!sub.content.contains("Cargo.toml"));
    }

    #[test]
    fn fuzzy_find_ranks_workspace_files() {
        let root = temp_root("fuzzy");
        touch(&root, "src/main.rs", "x");
        touch(&root, "src/lib.rs", "x");
        touch(&root, "docs/guide.md", "x");
        let result = fuzzy_find(&root, "call_1", "main");
        assert!(result.ok, "{}", result.content);
        assert!(result.content.contains("src/main.rs"));
        assert!(!result.content.contains("guide.md"));
        let none = fuzzy_find(&root, "call_1", "zzzznope");
        assert!(none.ok, "no match is an empty success, not an error");
        assert!(none.content.contains("no files match"));
    }

    #[test]
    fn search_files_finds_hits_case_insensitively() {
        let root = temp_root("search");
        touch(&root, "src/a.rs", "fn needle() {}\n");
        touch(&root, "src/b.rs", "nothing here\n");
        let result = search_files(&root, "call_1", "NEEDLE");
        assert!(result.ok, "{}", result.content);
        assert!(result.content.contains("src/a.rs:1"), "{}", result.content);
        assert!(result.content.contains("fn needle"));
        assert!(!result.content.contains("b.rs"));
        let none = search_files(&root, "call_1", "absent-string");
        assert!(none.ok, "no hits is an empty success, not an error");
        assert!(none.content.contains("no matches"));
    }

    // -- §17.2 output truncation -------------------------------------------------

    #[test]
    fn s17_2_output_is_truncated_with_notice_at_256kb() {
        let text = "x".repeat(MAX_OUTPUT_BYTES + 1024);
        let (capped, truncated) = cap_output(&text, MAX_OUTPUT_BYTES);
        assert!(truncated);
        assert!(capped.contains("[output truncated at 256KB]"));
        assert!(capped.len() < MAX_OUTPUT_BYTES + 64);
        let (same, truncated) = cap_output("short", MAX_OUTPUT_BYTES);
        assert_eq!(same, "short");
        assert!(!truncated);
        // The cut lands on a UTF-8 boundary.
        let multi = "é".repeat(MAX_OUTPUT_BYTES / 2 + 10); // 2 bytes per é
        let (capped, _) = cap_output(&multi, MAX_OUTPUT_BYTES);
        assert!(capped.is_char_boundary(MAX_OUTPUT_BYTES) || capped.len() < MAX_OUTPUT_BYTES + 32);
    }

    // -- write planning (§17.1/§17.4) --------------------------------------------

    #[test]
    fn plan_write_create_diff_is_all_additions() {
        let root = temp_root("write-create");
        let (rel, diff) = plan_write(&root, "new.rs", "fn main() {}\n").expect("plan");
        assert_eq!(rel, "new.rs");
        assert!(diff.contains("--- /dev/null"));
        assert!(diff.contains("+++ b/new.rs"));
        assert!(diff.contains("@@ -0,0 +1,1 @@"));
        assert!(diff.contains("+fn main() {}"));
        assert!(!diff.contains("-fn"));
        // Nothing was written — a plan, not an execution.
        assert!(!root.join("new.rs").exists());
    }

    #[test]
    fn plan_write_overwrite_diff_shows_both_sides() {
        let root = temp_root("write-over");
        touch(&root, "f.txt", "alpha\nbeta\ngamma\n");
        let (_, diff) = plan_write(&root, "f.txt", "alpha\nBETA\ngamma\n").expect("plan");
        assert!(diff.contains("--- a/f.txt"));
        assert!(diff.contains("+++ b/f.txt"));
        assert!(diff.contains("-beta"));
        assert!(diff.contains("+BETA"));
        // The file is untouched — approving is the save moment (§17.4).
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "alpha\nbeta\ngamma\n"
        );
    }

    #[test]
    fn plan_write_refuses_oversize_content_and_dirs() {
        let root = temp_root("write-cap");
        let huge = "x".repeat(MAX_READ_BYTES as usize + 1);
        let err = plan_write(&root, "big.txt", &huge).expect_err("cap");
        assert!(err.contains("4MB write cap"), "got: {err}");
        touch(&root, "d/x", "");
        let err = plan_write(&root, "d", "content").expect_err("dir");
        assert!(err.contains("directory"));
        let err = plan_write(&root, ".", "content").expect_err("root");
        assert!(err.contains("workspace root"));
    }

    #[test]
    fn plan_write_refuses_empty_content_for_a_non_empty_target() {
        let root = temp_root("write-empty");
        touch(&root, "f.txt", "keep me\n");
        let err = plan_write(&root, "f.txt", "").expect_err("empty overwrite");
        assert!(err.contains("empty content"), "got: {err}");
        assert!(err.contains("delete_file"), "got: {err}");
        // Planning never writes: the bytes are still there.
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "keep me\n"
        );
        // An already-empty target and a brand-new file may be empty.
        touch(&root, "empty.txt", "");
        assert!(plan_write(&root, "empty.txt", "").is_ok());
        assert!(plan_write(&root, "new-empty.txt", "").is_ok());
    }

    #[test]
    fn apply_write_refuses_empty_content_without_touching_the_file() {
        let root = temp_root("apply-write-empty");
        touch(&root, "f.txt", "keep me\n");
        let result = apply_write(&root, "call_1", "f.txt", "");
        assert!(!result.ok, "{}", result.content);
        assert!(result.content.contains("delete_file"), "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "keep me\n"
        );
        // No backup and no temp: nothing was replaced.
        assert!(!root.join(".f.txt.skarn-bak").exists());
    }

    #[test]
    fn write_atomic_keeps_previous_content_in_a_sidecar() {
        let root = temp_root("write-backup");
        let target = root.join("f.txt");
        std::fs::write(&target, "old\n").expect("seed");
        let backup = write_atomic(&target, "new\n")
            .expect("write")
            .expect("existing target gets a backup");
        assert_eq!(backup, root.join(".f.txt.skarn-bak"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "old\n");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\n");
        // The next write replaces the backup with the next-most-recent bytes.
        let backup = write_atomic(&target, "newer\n")
            .expect("write")
            .expect("backup");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "new\n");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "newer\n");
        // A create has nothing to preserve.
        assert!(write_atomic(&root.join("fresh.txt"), "x\n")
            .expect("create")
            .is_none());
    }

    #[test]
    fn plan_edit_validates_the_exact_match() {
        let root = temp_root("edit-plan");
        touch(&root, "f.txt", "fn main() {}\nfn other() {}\n");
        let err = plan_edit(&root, "f.txt", "absent", "x", false).expect_err("missing");
        assert!(err.contains("not found"));
        touch(&root, "dup.txt", "same\nsame\n");
        let err = plan_edit(&root, "dup.txt", "same", "x", false).expect_err("ambiguous");
        assert!(err.contains("2 times") || err.contains("occurs"));
        let (rel, diff) = plan_edit(&root, "dup.txt", "same", "x", true).expect("replace_all");
        assert_eq!(rel, "dup.txt");
        assert!(diff.contains("-same"));
        assert!(diff.contains("+x"));
        let err = plan_edit(&root, "f.txt", "fn", "fn", false).expect_err("no-op");
        assert!(err.contains("identical"));
        let err = plan_edit(&root, "missing.txt", "a", "b", false).expect_err("missing file");
        assert!(err.contains("not an existing file"));
    }

    // -- apply (§17.4: only after the user's approval) ---------------------------

    #[test]
    fn apply_write_creates_and_overwrites_atomically() {
        let root = temp_root("apply-write");
        let result = apply_write(&root, "call_1", "nested/new.rs", "content\n");
        assert!(result.ok, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("nested/new.rs")).unwrap(),
            "content\n"
        );
        // No temp file left behind.
        assert!(!root.join("nested/.new.rs.skarn-tmp").exists());
        let result = apply_write(&root, "call_1", "nested/new.rs", "replaced\n");
        assert!(result.ok);
        assert_eq!(
            std::fs::read_to_string(root.join("nested/new.rs")).unwrap(),
            "replaced\n"
        );
        // The approved overwrite preserved the bytes the user was viewing.
        assert_eq!(
            std::fs::read_to_string(root.join("nested/.new.rs.skarn-bak")).unwrap(),
            "content\n"
        );
    }

    #[test]
    fn write_atomic_temp_names_are_unique_siblings() {
        let root = temp_root("write-tmp-unique");
        let target = root.join("f.txt");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let tmp = temp_sibling(&target);
            assert_eq!(tmp.parent(), target.parent(), "temp must be a sibling");
            let name = tmp
                .file_name()
                .expect("temp name")
                .to_string_lossy()
                .into_owned();
            assert!(
                name.starts_with(".f.txt.skarn-tmp-"),
                "unexpected temp name: {name}"
            );
            assert!(
                seen.insert(tmp.clone()),
                "repeated temp path: {}",
                tmp.display()
            );
        }
    }

    #[test]
    fn concurrent_writes_to_the_same_target_do_not_collide() {
        let root = temp_root("write-concurrent");
        let target = root.join("shared.txt");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();
        for writer in 0..8 {
            let target = target.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let content = format!("writer {writer}\n");
                barrier.wait();
                for _ in 0..20 {
                    let _ = write_atomic(&target, &content)?;
                }
                Ok::<(), std::io::Error>(())
            }));
        }
        for handle in handles {
            handle
                .join()
                .expect("writer thread panicked")
                .expect("concurrent write_atomic failed (temp collision?)");
        }
        let final_content = std::fs::read_to_string(&target).expect("target exists");
        assert!(
            final_content.starts_with("writer "),
            "unexpected final content: {final_content}"
        );
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .expect("read fixture root")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".skarn-tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn apply_write_rejects_escape_even_when_called_directly() {
        let root = temp_root("apply-escape");
        let result = apply_write(&root, "call_1", "../out.txt", "x");
        assert!(!result.ok);
        assert!(result.content.contains("escapes"));
        assert!(!root.join("../out.txt").exists());
    }

    #[test]
    fn apply_edit_replaces_and_refuses_changed_files() {
        let root = temp_root("apply-edit");
        touch(&root, "f.txt", "alpha\nbeta\ngamma\n");
        let result = apply_edit(&root, "call_1", "f.txt", "beta", "BETA", false);
        assert!(result.ok, "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
        // If the file changed since approval, the edit is refused.
        touch(&root, "g.txt", "one\ntwo\n");
        std::fs::write(root.join("g.txt"), "completely different\n").unwrap();
        let stale = apply_edit(&root, "call_1", "g.txt", "two", "2", false);
        assert!(!stale.ok);
        assert!(stale.content.contains("changed since"));
        assert_eq!(
            std::fs::read_to_string(root.join("g.txt")).unwrap(),
            "completely different\n"
        );
        // replace_all
        touch(&root, "h.txt", "x x x\n");
        let result = apply_edit(&root, "call_1", "h.txt", "x", "y", true);
        assert!(result.ok);
        assert_eq!(
            std::fs::read_to_string(root.join("h.txt")).unwrap(),
            "y y y\n"
        );
    }

    #[test]
    fn apply_delete_removes_files_refuses_dirs_and_root() {
        let root = temp_root("apply-delete");
        touch(&root, "doomed.txt", "x");
        let result = apply_delete(&root, "call_1", "doomed.txt");
        assert!(result.ok, "{}", result.content);
        assert!(!root.join("doomed.txt").exists());
        touch(&root, "d/keep.txt", "x");
        let dir = apply_delete(&root, "call_1", "d");
        assert!(!dir.ok);
        assert!(dir.content.contains("directory"));
        assert!(root.join("d/keep.txt").exists());
        let root_itself = apply_delete(&root, "call_1", ".");
        assert!(!root_itself.ok);
    }

    // -- unified diff -------------------------------------------------------------

    #[test]
    fn diff_identical_content_has_no_hunks() {
        let diff = unified_diff("f.txt", "a\nb\n", "a\nb\n");
        assert!(!diff.contains("@@"));
    }

    #[test]
    fn diff_distant_changes_form_separate_hunks() {
        let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let mut new_lines: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        new_lines[2] = "CHANGED-A".to_owned();
        new_lines[35] = "CHANGED-B".to_owned();
        let new = new_lines.join("\n");
        let diff = unified_diff("f.txt", &old, &new);
        // 33 unchanged lines between the two changes (>> 2*3 context)
        // → two separate hunks.
        assert_eq!(
            diff.matches("@@ -").count(),
            2,
            "two hunks expected:\n{diff}"
        );
        assert!(diff.contains("-line 2"));
        assert!(diff.contains("+CHANGED-A"));
        assert!(diff.contains("-line 35"));
        assert!(diff.contains("+CHANGED-B"));
    }

    #[test]
    fn diff_truncates_at_max_lines() {
        let old = (0..MAX_DIFF_LINES)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..MAX_DIFF_LINES)
            .map(|i| format!("LINE {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let diff = unified_diff("big.txt", &old, &new);
        assert!(diff.contains("diff truncated"));
    }

    #[test]
    fn diff_deletion_and_no_trailing_newline() {
        let diff = unified_diff("f.txt", "keep\ndrop\nkeep\n", "keep\nkeep\n");
        assert!(diff.contains("-drop"));
        let diff = unified_diff("f.txt", "no newline", "no newline!");
        assert!(diff.contains("+no newline!"));
    }

    #[test]
    fn cas_mutations_reject_mid_flight_disk_drift_without_touching_target() {
        let root = temp_root("cas-drift");
        touch(&root, "cas.txt", "before\n");
        let resolved = resolve_rooted(&root, "cas.txt").unwrap();
        let pre = capture_file_precondition(&resolved).unwrap();
        assert!(pre.existed);

        // Simulate human edit on disk after planning/approval.
        std::fs::write(&resolved, "human edit\n").unwrap();

        let write_res = apply_write_cas(&root, "c1", "cas.txt", "agent\n", Some(&pre), false);
        assert!(!write_res.tool_result.ok);
        assert!(write_res.file_change.is_none());
        assert_eq!(std::fs::read_to_string(&resolved).unwrap(), "human edit\n");

        let edit_res =
            apply_edit_cas(&root, "c2", "cas.txt", "human", "agent", false, Some(&pre), false);
        assert!(!edit_res.tool_result.ok);
        assert!(edit_res.file_change.is_none());
        assert_eq!(std::fs::read_to_string(&resolved).unwrap(), "human edit\n");

        let del_res = apply_delete_cas(&root, "c3", "cas.txt", Some(&pre));
        assert!(!del_res.tool_result.ok);
        assert!(del_res.file_change.is_none());
        assert!(resolved.exists());

        // Fresh precondition succeeds and leaves no .skarn-bak when write_backup == false.
        let fresh = capture_file_precondition(&resolved).unwrap();
        let ok_edit = apply_edit_cas(
            &root,
            "c4",
            "cas.txt",
            "human",
            "agent",
            false,
            Some(&fresh),
            false,
        );
        assert!(ok_edit.tool_result.ok);
        assert!(ok_edit.file_change.is_some());
        assert!(!backup_sibling(&resolved).exists());
    }
}
