//! A one-file diff for the git panel: HEAD → index (staged) or index →
//! working tree (unstaged).
//!
//! Structured, not text: the shell can paint it in any read-only surface
//! later. Origin letters are git's own: `'+'` added, `'-'` removed,
//! `' '` context, `'\\'` "no newline at end of file".
//!
//! Both bounds below keep a large or minified file from turning one diff into
//! an unbounded allocation (and an unbounded IPC payload): the collector stops
//! at the cap and reports [`FileDiff::truncated`].

use std::path::PathBuf;

/// The most lines one [`FileDiff`] may carry.
pub const MAX_DIFF_LINES: usize = 20_000;
/// The most bytes one [`FileDiff`] may carry across its lines.
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

/// The diff of a single file on one side of a row: HEAD → index
/// ([`crate::GitRepo::diff_index`]) or index → working tree
/// ([`crate::GitRepo::diff_workdir`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Repo-relative path with `/` separators.
    pub path: PathBuf,
    pub hunks: Vec<Hunk>,
    /// Whether [`MAX_DIFF_LINES`] / [`MAX_DIFF_BYTES`] cut the patch short.
    /// [`FileDiff::to_unified`] then ends with a visible notice, so a partial
    /// diff is never mistaken for the whole change.
    pub truncated: bool,
}

impl FileDiff {
    /// Whether any hunk has content.
    pub fn is_empty(&self) -> bool {
        self.hunks.iter().all(|hunk| hunk.lines.is_empty())
    }

    /// The unified-diff text (Phase R6b: the git panel's read-only diff
    /// view) — `--- a/<path>` / `+++ b/<path>` headers, `@@` hunks, and
    /// the lines with their origin markers. The same shape
    /// `agent_cards`' parser consumes (`parse_unified_diff`), so the
    /// DiffView widget paints it directly.
    pub fn to_unified(&self) -> String {
        let mut out = String::new();
        if self.is_empty() && !self.truncated {
            return out;
        }
        out.push_str(&format!("--- a/{}\n", self.path.to_string_lossy()));
        out.push_str(&format!("+++ b/{}\n", self.path.to_string_lossy()));
        for hunk in &self.hunks {
            out.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
            ));
            for line in &hunk.lines {
                match line.origin {
                    '\\' => out.push_str("\\ No newline at end of file\n"),
                    other => out.push_str(&format!("{other}{}\n", line.content)),
                }
            }
        }
        if self.truncated {
            out.push_str("[diff truncated: the file is too large or binary to show in full]\n");
        }
        out
    }
}

/// One hunk: the `@@ -old_start,old_lines +new_start,new_lines @@` range
/// plus its lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

/// One diff line. `old_no` / `new_no` are the 1-based line numbers on the
/// respective side (`None` for added / removed / meta lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// `'+` `'-'` `' '` `'\\'`.
    pub origin: char,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The line text without the trailing newline.
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unified_announces_a_truncated_patch() {
        let diff = FileDiff {
            path: PathBuf::from("big.min.js"),
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec![DiffLine {
                    origin: '+',
                    old_no: None,
                    new_no: Some(1),
                    content: "x".into(),
                }],
            }],
            truncated: true,
        };
        assert!(diff
            .to_unified()
            .ends_with("[diff truncated: the file is too large or binary to show in full]\n"));
    }

    #[test]
    fn to_unified_renders_headers_hunks_and_lines() {
        let diff = FileDiff {
            path: PathBuf::from("src/main.rs"),
            hunks: vec![Hunk {
                old_start: 10,
                old_lines: 1,
                new_start: 10,
                new_lines: 2,
                lines: vec![
                    DiffLine {
                        origin: '-',
                        old_no: Some(10),
                        new_no: None,
                        content: "fn main() {}".into(),
                    },
                    DiffLine {
                        origin: '+',
                        old_no: None,
                        new_no: Some(10),
                        content: "fn main() -> u32 {".into(),
                    },
                ],
            }],
            truncated: false,
        };
        let text = diff.to_unified();
        assert_eq!(
            text,
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -10,1 +10,2 @@\n-fn main() {}\n+fn main() -> u32 {\n"
        );
        // An empty diff renders nothing.
        assert_eq!(FileDiff::default().to_unified(), "");
    }
}
