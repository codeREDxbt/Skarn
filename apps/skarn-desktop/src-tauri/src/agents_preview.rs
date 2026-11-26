//! Read-only, workspace-scoped previews for the detached Agents window.

use crate::{state::DesktopState, workspace::Workspace};
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const PREVIEW_BYTES: usize = 256 * 1024;

#[derive(Debug, Serialize)]
pub struct AgentsFilePreviewDto {
    workspace_id: String,
    path: String,
    text: Option<String>,
    source: &'static str,
    dirty: bool,
    version: Option<u64>,
    byte_length: u64,
    truncated: bool,
    binary: bool,
}

#[derive(Debug, Serialize)]
pub struct AgentsGitDiffDto {
    workspace_id: String,
    path: String,
    baseline: String,
    unified_diff: String,
    truncated: bool,
}

fn relative(path: &str) -> Result<String, String> {
    let path = path.replace('\\', "/");
    skarn_protocol::validate_workspace_relative_path(&path).map_err(|e| e.to_string())?;
    if path.len() > 4096
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.eq_ignore_ascii_case(".git")
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        })
    {
        return Err("Preview requires a valid workspace-relative file path.".into());
    }
    Ok(path)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    let link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        link || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        link
    }
}

/// Retain ancestor handles on Windows so checked directories cannot be replaced
/// with junctions between validation and reading. Never follow a reparse point.
fn pin_ancestors(path: &Path) -> Result<Vec<fs::File>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let mut pins = Vec::new();
        for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let handle = fs::OpenOptions::new()
                .access_mode(1)
                .share_mode(3)
                .custom_flags(0x02000000 | 0x00200000)
                .open(ancestor)
                .map_err(|e| e.to_string())?;
            if is_link(&handle.metadata().map_err(|e| e.to_string())?) {
                return Err("Previews cannot traverse symbolic links or junctions.".into());
            }
            pins.push(handle);
        }
        Ok(pins)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(Vec::new())
    }
}

fn checked_path(root: &Path, relative: &str, allow_missing: bool) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if is_link(&metadata) {
                    return Err("Previews cannot traverse symbolic links or junctions.".into());
                }
                if index + 1 < parts.len() && !metadata.is_dir() {
                    return Err("A preview ancestor is not a directory.".into());
                }
                let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
                if !canonical.starts_with(root) {
                    return Err("The preview path is outside the workspace.".into());
                }
                path = canonical;
            }
            Err(e) if allow_missing && e.kind() == std::io::ErrorKind::NotFound => {
                for remaining in &parts[index + 1..] {
                    path.push(remaining);
                }
                return Ok(path);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}

fn capped(text: &str) -> (String, bool) {
    let mut end = text.len().min(PREVIEW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), end < text.len())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        let left = left.to_string_lossy().replace('\\', "/");
        let right = right.to_string_lossy().replace('\\', "/");
        left.strip_prefix("//?/")
            .unwrap_or(&left)
            .eq_ignore_ascii_case(right.strip_prefix("//?/").unwrap_or(&right))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn file_preview(
    workspace: &Workspace,
    workspace_id: &str,
    path: &str,
) -> Result<AgentsFilePreviewDto, String> {
    let path = relative(path)?;
    if let Some(buffer) = workspace
        .buffers()
        .into_iter()
        .find(|buffer| paths_equal(Path::new(buffer.path.as_str()), Path::new(&path)))
    {
        let (text, truncated) = capped(&buffer.text);
        return Ok(AgentsFilePreviewDto {
            workspace_id: workspace_id.into(),
            path,
            byte_length: buffer.text.len() as u64,
            text: Some(text),
            source: "buffer",
            dirty: buffer.dirty,
            version: Some(buffer.version),
            truncated,
            binary: false,
        });
    }
    let target = checked_path(workspace.root(), &path, false)?;
    let _pins = pin_ancestors(target.parent().ok_or("Missing preview parent.")?)?;
    let target = checked_path(workspace.root(), &path, false)?;
    if !fs::symlink_metadata(&target)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("Only regular files can be previewed.".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x00200000);
    }
    let file = options.open(target).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if is_link(&metadata) || !metadata.is_file() {
        return Err("Only regular files can be previewed.".into());
    }
    let mut bytes = Vec::new();
    file.take((PREVIEW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let truncated = bytes.len() > PREVIEW_BYTES;
    bytes.truncate(PREVIEW_BYTES);
    let binary = bytes.contains(&0);
    let text = if binary {
        None
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(error) if truncated && error.error_len().is_none() => Some(
                String::from_utf8(bytes[..error.valid_up_to()].to_vec())
                    .map_err(|e| e.to_string())?,
            ),
            Err(_) => None,
        }
    };
    Ok(AgentsFilePreviewDto {
        workspace_id: workspace_id.into(),
        path,
        binary: binary || text.is_none(),
        text,
        source: "disk",
        dirty: false,
        version: None,
        byte_length: metadata.len(),
        truncated,
    })
}

fn git_preview(
    root: &Path,
    workspace_id: &str,
    path: &str,
    baseline: &str,
) -> Result<AgentsGitDiffDto, String> {
    let path = relative(path)?;
    if !matches!(baseline, "index" | "working_tree") {
        return Err("Unknown Git preview baseline.".into());
    }
    // selora-git uses libgit2 pathspecs. Reject magic rather than allowing one
    // filename to match other files; ordinary filenames remain exact paths.
    if path.contains(['[', ']']) {
        return Err("Git previews do not support pathspec characters in filenames.".into());
    }
    let target = checked_path(root, &path, true)?;
    let existing_parent = target
        .ancestors()
        .skip(1)
        .find(|p| p.is_dir())
        .ok_or("Missing preview parent.")?;
    let _pins = pin_ancestors(existing_parent)?;
    let target = checked_path(root, &path, true)?;
    let repo = skarn_git::open_repo(root).map_err(|e| e.to_string())?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) if !metadata.is_file() || is_link(&metadata) => {
            return Err("Git previews require a single regular file.".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A nonexistent directory could otherwise become a recursive
            // pathspec. Only accept an exact deleted path from real Git status.
            let status = repo.status().map_err(|e| e.to_string())?;
            if !status
                .entries
                .iter()
                .any(|entry| paths_equal(&entry.abs_path, &target))
                && !status
                    .conflicts
                    .iter()
                    .any(|conflict| paths_equal(conflict, &target))
            {
                return Err("The deleted path is not present in Git status.".into());
            }
        }
        Err(error) => return Err(error.to_string()),
        _ => {}
    }
    let diff = match baseline {
        "index" => repo.diff_index(&target),
        _ => repo.diff_workdir(&target),
    }
    .map_err(|e| e.to_string())?;
    let (unified_diff, truncated) = capped(&diff.to_unified());
    Ok(AgentsGitDiffDto {
        workspace_id: workspace_id.into(),
        path,
        baseline: baseline.into(),
        unified_diff,
        truncated: truncated || diff.truncated,
    })
}

#[tauri::command]
pub async fn agents_read_file_preview(
    state: tauri::State<'_, DesktopState>,
    workspace_id: String,
    path: String,
) -> Result<AgentsFilePreviewDto, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.explorer_with_workspace(&workspace_id, |workspace| {
            file_preview(workspace, &workspace_id, &path)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn agents_read_git_diff(
    state: tauri::State<'_, DesktopState>,
    workspace_id: String,
    path: String,
    baseline: String,
) -> Result<AgentsGitDiffDto, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.explorer_with_workspace(&workspace_id, |workspace| {
            git_preview(workspace.root(), &workspace_id, &path, &baseline)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths_and_baselines() {
        for path in [
            "../secret",
            "/secret",
            "C:\\secret",
            ".git/config",
            "file:stream",
            "dir/../secret",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(git_preview(dir.path(), "w", "a.txt", "HEAD").is_err());
    }

    #[test]
    fn disk_read_is_bounded_and_binary_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("large.txt"), "é".repeat(PREVIEW_BYTES)).unwrap();
        fs::write(dir.path().join("binary"), [0, 1, 2]).unwrap();
        let workspace = Workspace::open(dir.path().to_path_buf()).unwrap();
        let preview = file_preview(&workspace, "w", "large.txt").unwrap();
        assert!(preview.truncated);
        assert!(preview.text.unwrap().len() <= PREVIEW_BYTES);
        let preview = file_preview(&workspace, "w", "binary").unwrap();
        assert!(preview.binary);
        assert!(preview.text.is_none());
        assert!(workspace.buffers().is_empty());
    }

    #[test]
    fn open_buffer_wins_over_disk_without_changing_active_buffer() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "buffer text").unwrap();
        let workspace = Workspace::open(dir.path().to_path_buf()).unwrap();
        let buffer = workspace.open_file("a.txt").unwrap();
        workspace
            .apply_change(
                &buffer.buffer_id,
                buffer.version,
                &[skarn_protocol::TextChange {
                    start: skarn_protocol::TextPosition {
                        line: 0,
                        utf16_column: 0,
                    },
                    end: skarn_protocol::TextPosition {
                        line: 0,
                        utf16_column: 0,
                    },
                    replacement: "dirty ".into(),
                }],
            )
            .unwrap();
        fs::write(dir.path().join("a.txt"), "disk text").unwrap();
        let active = workspace.active_buffer_id();
        let preview = file_preview(&workspace, "w", "a.txt").unwrap();
        assert_eq!(preview.source, "buffer");
        assert_eq!(preview.text.as_deref(), Some("dirty buffer text"));
        assert_eq!(preview.version, Some(buffer.version + 1));
        assert!(preview.dirty);
        assert_eq!(workspace.active_buffer_id(), active);
    }

    #[test]
    fn staged_and_working_tree_diffs_are_distinct_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let raw = git2::Repository::init(dir.path()).unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("a.txt"), "base\n").unwrap();
        let mut index = raw.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree = raw.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Test", "test@example.com").unwrap();
        raw.commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .unwrap();
        fs::write(root.join("a.txt"), "staged\n").unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();
        fs::write(root.join("a.txt"), "working\n").unwrap();
        let staged = git_preview(&root, "w", "a.txt", "index").unwrap();
        let working = git_preview(&root, "w", "a.txt", "working_tree").unwrap();
        assert!(staged.unified_diff.contains("+staged"));
        assert!(!staged.unified_diff.contains("+working"));
        assert!(working.unified_diff.contains("+working"));
        fs::create_dir(root.join("directory")).unwrap();
        assert!(git_preview(&root, "w", "directory", "working_tree").is_err());
        fs::write(root.join("a.txt"), "long line\n".repeat(40_000)).unwrap();
        let large = git_preview(&root, "w", "a.txt", "working_tree").unwrap();
        assert!(large.truncated);
        assert!(large.unified_diff.len() <= PREVIEW_BYTES);
        fs::remove_file(root.join("a.txt")).unwrap();
        assert!(git_preview(&root, "w", "a.txt", "working_tree")
            .unwrap()
            .unified_diff
            .contains("-staged"));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_directory_junction() {
        use std::os::windows::fs::symlink_dir;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        if symlink_dir(outside.path(), dir.path().join("link")).is_err() {
            return;
        }
        let root = fs::canonicalize(dir.path()).unwrap();
        assert!(checked_path(&root, "link/file.txt", true).is_err());
    }
}
