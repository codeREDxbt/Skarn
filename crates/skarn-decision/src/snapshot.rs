use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::types::{DocumentVersionRecord, WorkspaceSnapshot};
use crate::validator::{canonical_root, rel_display, resolve_rooted};

/// Lowercase hex SHA-256 of an in-memory byte slice.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Lowercase hex SHA-256 of a file on disk (`Ok(None)` if not found).
pub fn sha256_file(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(sha256_bytes(&bytes))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BufferSnapshotState {
    pub revision: u64,
    pub dirty: bool,
}

pub trait WorkspaceStateReader: Send + Sync {
    fn workspace_root(&self) -> &Path;
    fn git_head(&self) -> Option<String>;
    fn workspace_trusted(&self) -> bool;
    fn trust_epoch(&self) -> u64;
    fn local_only(&self) -> bool;
    fn buffer_state(&self, rel_path: &str) -> Option<BufferSnapshotState>;

    fn read_disk_file(&self, rel_path: &str) -> std::io::Result<Option<Vec<u8>>> {
        let resolved = resolve_rooted(self.workspace_root(), rel_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.message))?;
        match std::fs::read(resolved) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// Thread-safe filesystem-backed [`WorkspaceStateReader`] with mutable open-buffer,
/// trust-epoch, and `local_only` state for standalone use and concurrency tests.
pub struct FilesystemWorkspaceReader {
    root: PathBuf,
    git_head: RwLock<Option<String>>,
    trusted: AtomicBool,
    trust_epoch: AtomicU64,
    local_only: AtomicBool,
    buffers: RwLock<BTreeMap<String, BufferSnapshotState>>,
}

impl FilesystemWorkspaceReader {
    pub fn new(root: impl Into<PathBuf>, workspace_trusted: bool, local_only: bool) -> Self {
        Self {
            root: canonical_root(&root.into()),
            git_head: RwLock::new(None),
            trusted: AtomicBool::new(workspace_trusted),
            trust_epoch: AtomicU64::new(1),
            local_only: AtomicBool::new(local_only),
            buffers: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn set_trusted(&self, trusted: bool) -> u64 {
        self.trusted.store(trusted, Ordering::SeqCst);
        self.trust_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn set_local_only(&self, local_only: bool) {
        self.local_only.store(local_only, Ordering::SeqCst);
    }

    pub fn set_git_head(&self, head: Option<String>) {
        if let Ok(mut guard) = self.git_head.write() {
            *guard = head;
        }
    }

    pub fn set_buffer_state(&self, rel_path: impl Into<String>, state: BufferSnapshotState) {
        let normalized = normalize_rel(&rel_path.into());
        if let Ok(mut guard) = self.buffers.write() {
            guard.insert(normalized, state);
        }
    }

    pub fn clear_buffer_state(&self, rel_path: &str) {
        let normalized = normalize_rel(rel_path);
        if let Ok(mut guard) = self.buffers.write() {
            guard.remove(&normalized);
        }
    }
}

impl WorkspaceStateReader for FilesystemWorkspaceReader {
    fn workspace_root(&self) -> &Path {
        &self.root
    }

    fn git_head(&self) -> Option<String> {
        self.git_head.read().ok().and_then(|g| g.clone())
    }

    fn workspace_trusted(&self) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }

    fn trust_epoch(&self) -> u64 {
        self.trust_epoch.load(Ordering::SeqCst)
    }

    fn local_only(&self) -> bool {
        self.local_only.load(Ordering::SeqCst)
    }

    fn buffer_state(&self, rel_path: &str) -> Option<BufferSnapshotState> {
        let normalized = normalize_rel(rel_path);
        self.buffers
            .read()
            .ok()
            .and_then(|g| g.get(&normalized).cloned())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum DriftReason {
    #[error("open editor buffer is dirty for `{path}`")]
    OpenBufferDirty { path: String },
    #[error(
        "open editor buffer revision drifted for `{path}` (expected {expected:?}, actual {actual:?})"
    )]
    BufferRevisionMismatch {
        path: String,
        expected: Option<u64>,
        actual: Option<u64>,
    },
    #[error("file existence drifted on disk for `{path}` (expected {expected}, actual {actual})")]
    FileExistenceMismatch {
        path: String,
        expected: bool,
        actual: bool,
    },
    #[error("file content sha256 drifted on disk for `{path}`")]
    FileHashMismatch {
        path: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("workspace trust drifted (expected trusted={expected_trusted} epoch={expected_epoch}, actual trusted={actual_trusted} epoch={actual_epoch})")]
    TrustDrifted {
        expected_trusted: bool,
        expected_epoch: u64,
        actual_trusted: bool,
        actual_epoch: u64,
    },
    #[error("git HEAD drifted (expected {expected:?}, actual {actual:?})")]
    GitHeadMismatch {
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("path `{path}` failed workspace containment: {reason}")]
    PathInvalid { path: String, reason: String },
    #[error("I/O error reading `{path}`: {message}")]
    IoError { path: String, message: String },
}

pub struct SnapshotCapturer;

impl SnapshotCapturer {
    pub fn capture(
        reader: &dyn WorkspaceStateReader,
        target_paths: &[&str],
        captured_at_ms: u64,
    ) -> Result<WorkspaceSnapshot, DriftReason> {
        let root = canonical_root(reader.workspace_root());
        let mut document_versions = BTreeMap::new();

        for raw_path in target_paths {
            let resolved = resolve_rooted(&root, raw_path).map_err(|e| DriftReason::PathInvalid {
                path: (*raw_path).to_owned(),
                reason: e.message,
            })?;
            let rel = rel_display(&root, &resolved);
            let bytes = reader
                .read_disk_file(&rel)
                .map_err(|e| DriftReason::IoError {
                    path: rel.clone(),
                    message: e.to_string(),
                })?;
            let buf = reader.buffer_state(&rel);
            let record = match bytes {
                Some(data) => DocumentVersionRecord {
                    revision: buf.as_ref().map(|b| b.revision),
                    sha256: Some(sha256_bytes(&data)),
                    dirty: buf.as_ref().is_some_and(|b| b.dirty),
                    exists: true,
                },
                None => DocumentVersionRecord {
                    revision: buf.as_ref().map(|b| b.revision),
                    sha256: None,
                    dirty: buf.as_ref().is_some_and(|b| b.dirty),
                    exists: false,
                },
            };
            document_versions.insert(rel, record);
        }

        Ok(WorkspaceSnapshot {
            snapshot_id: format!("snap-{}", Uuid::new_v4()),
            workspace_root: root.to_string_lossy().into_owned(),
            git_head: reader.git_head(),
            document_versions,
            workspace_trusted: reader.workspace_trusted(),
            trust_epoch: reader.trust_epoch(),
            local_only: reader.local_only(),
            captured_at_ms,
        })
    }
}

pub struct SnapshotRechecker;

impl SnapshotRechecker {
    pub fn recheck(
        snapshot: &WorkspaceSnapshot,
        reader: &dyn WorkspaceStateReader,
        target_paths: &[&str],
        require_clean_buffers: bool,
    ) -> Result<(), DriftReason> {
        let actual_trusted = reader.workspace_trusted();
        let actual_epoch = reader.trust_epoch();
        if actual_trusted != snapshot.workspace_trusted || actual_epoch != snapshot.trust_epoch {
            return Err(DriftReason::TrustDrifted {
                expected_trusted: snapshot.workspace_trusted,
                expected_epoch: snapshot.trust_epoch,
                actual_trusted,
                actual_epoch,
            });
        }

        let actual_head = reader.git_head();
        if actual_head != snapshot.git_head {
            return Err(DriftReason::GitHeadMismatch {
                expected: snapshot.git_head.clone(),
                actual: actual_head,
            });
        }

        let root = canonical_root(reader.workspace_root());
        let mut to_check = BTreeSet::new();
        for key in snapshot.document_versions.keys() {
            to_check.insert(key.clone());
        }
        for raw_path in target_paths {
            let resolved = resolve_rooted(&root, raw_path).map_err(|e| DriftReason::PathInvalid {
                path: (*raw_path).to_owned(),
                reason: e.message,
            })?;
            to_check.insert(rel_display(&root, &resolved));
        }

        for rel in to_check {
            // Re-verify path containment on live disk to defeat mid-flight symlink swaps.
            let _resolved = resolve_rooted(&root, &rel).map_err(|e| DriftReason::PathInvalid {
                path: rel.clone(),
                reason: e.message,
            })?;

            let expected = snapshot.document_versions.get(&rel).ok_or_else(|| {
                DriftReason::PathInvalid {
                    path: rel.clone(),
                    reason: "target path was not covered by snapshot".to_owned(),
                }
            })?;

            let live_buf = reader.buffer_state(&rel);
            let live_dirty = live_buf.as_ref().is_some_and(|b| b.dirty);
            let live_rev = live_buf.as_ref().map(|b| b.revision);

            if require_clean_buffers && live_dirty {
                return Err(DriftReason::OpenBufferDirty { path: rel });
            }
            if live_dirty != expected.dirty {
                return Err(DriftReason::OpenBufferDirty { path: rel });
            }
            if live_rev != expected.revision {
                return Err(DriftReason::BufferRevisionMismatch {
                    path: rel,
                    expected: expected.revision,
                    actual: live_rev,
                });
            }

            let live_bytes = reader
                .read_disk_file(&rel)
                .map_err(|e| DriftReason::IoError {
                    path: rel.clone(),
                    message: e.to_string(),
                })?;

            let (actual_exists, actual_sha) = match live_bytes {
                Some(data) => (true, Some(sha256_bytes(&data))),
                None => (false, None),
            };

            if actual_exists != expected.exists {
                return Err(DriftReason::FileExistenceMismatch {
                    path: rel,
                    expected: expected.exists,
                    actual: actual_exists,
                });
            }
            if actual_sha != expected.sha256 {
                return Err(DriftReason::FileHashMismatch {
                    path: rel,
                    expected: expected.sha256.clone(),
                    actual: actual_sha,
                });
            }
        }

        Ok(())
    }
}

fn normalize_rel(rel: &str) -> String {
    let trimmed = rel.trim().replace('\\', "/");
    let stripped = trimmed.strip_prefix("./").unwrap_or(&trimmed);
    if stripped.is_empty() {
        ".".to_owned()
    } else {
        stripped.to_owned()
    }
}
