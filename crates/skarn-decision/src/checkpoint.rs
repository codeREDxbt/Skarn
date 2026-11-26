use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::snapshot::{sha256_bytes, sha256_file, WorkspaceStateReader};
use crate::validator::{rel_display, resolve_rooted};

pub const MAX_CHECKPOINT_TURNS_PER_WORKSPACE: usize = 50;
pub const MAX_CHECKPOINT_BYTES_TOTAL: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileCheckpointRecord {
    pub rel_path: String,
    pub existed_before: bool,
    pub before_sha256: Option<String>,
    pub existed_after: bool,
    pub after_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TurnCheckpointManifest {
    pub workspace_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub created_at_ms: u64,
    pub files: Vec<FileCheckpointRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFileChange {
    pub rel_path: String,
    pub before_bytes: Option<Vec<u8>>,
    pub after_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointConflict {
    pub rel_path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PruneStats {
    pub pruned_turns: usize,
    pub pruned_blobs: usize,
    pub total_bytes_remaining: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint preflight detected {0:?} conflict(s); aborted without modifying disk")]
    Conflict(Vec<CheckpointConflict>),
    #[error("missing checkpoint manifest for {workspace_id}/{session_id}/{turn_id}")]
    MissingManifest {
        workspace_id: String,
        session_id: String,
        turn_id: String,
    },
    #[error("missing content-addressed blob {sha256}")]
    MissingBlob { sha256: String },
    #[error("invalid checkpoint identifier or path: {0}")]
    InvalidSegment(String),
    #[error("path containment error: {0}")]
    PathEscape(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub struct TurnCheckpointStore {
    base_dir: PathBuf,
    max_turns_per_workspace: usize,
    max_total_bytes: u64,
}

impl TurnCheckpointStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            max_turns_per_workspace: MAX_CHECKPOINT_TURNS_PER_WORKSPACE,
            max_total_bytes: MAX_CHECKPOINT_BYTES_TOTAL,
        }
    }

    pub fn with_limits(
        base_dir: impl Into<PathBuf>,
        max_turns_per_workspace: usize,
        max_total_bytes: u64,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            max_turns_per_workspace,
            max_total_bytes,
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn blobs_dir(&self) -> PathBuf {
        self.base_dir.join("blobs")
    }

    fn blob_path(&self, sha256: &str) -> Result<PathBuf, CheckpointError> {
        validate_segment(sha256)?;
        Ok(self.blobs_dir().join(sha256))
    }

    fn turn_dir(
        &self,
        workspace_id: &str,
        session_id: &str,
        turn_id: &str,
    ) -> Result<PathBuf, CheckpointError> {
        validate_segment(workspace_id)?;
        validate_segment(session_id)?;
        validate_segment(turn_id)?;
        if workspace_id == "blobs" {
            return Err(CheckpointError::InvalidSegment(
                "workspace_id cannot be `blobs`".to_owned(),
            ));
        }
        Ok(self
            .base_dir
            .join(workspace_id)
            .join(session_id)
            .join(turn_id))
    }

    fn store_blob(&self, bytes: &[u8]) -> Result<String, CheckpointError> {
        let hash = sha256_bytes(bytes);
        let path = self.blob_path(&hash)?;
        if !path.exists() {
            write_atomic_bytes(&path, bytes)?;
        }
        Ok(hash)
    }

    fn load_blob(&self, sha256: &str) -> Result<Vec<u8>, CheckpointError> {
        let path = self.blob_path(sha256)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(CheckpointError::MissingBlob {
                sha256: sha256.to_owned(),
            }),
            Err(e) => Err(CheckpointError::Io(e)),
        }
    }

    pub fn load_manifest(
        &self,
        workspace_id: &str,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnCheckpointManifest, CheckpointError> {
        let manifest_path = self
            .turn_dir(workspace_id, session_id, turn_id)?
            .join("manifest.json");
        match std::fs::read(&manifest_path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(CheckpointError::MissingManifest {
                    workspace_id: workspace_id.to_owned(),
                    session_id: session_id.to_owned(),
                    turn_id: turn_id.to_owned(),
                })
            }
            Err(e) => Err(CheckpointError::Io(e)),
        }
    }

    pub fn record_turn(
        &self,
        workspace_id: &str,
        session_id: &str,
        turn_id: &str,
        created_at_ms: u64,
        changes: &[TurnFileChange],
    ) -> Result<TurnCheckpointManifest, CheckpointError> {
        let turn_dir = self.turn_dir(workspace_id, session_id, turn_id)?;
        let mut files = Vec::with_capacity(changes.len());

        for change in changes {
            let (existed_before, before_sha256) = match &change.before_bytes {
                Some(b) => (true, Some(self.store_blob(b)?)),
                None => (false, None),
            };
            let (existed_after, after_sha256) = match &change.after_bytes {
                Some(b) => (true, Some(self.store_blob(b)?)),
                None => (false, None),
            };
            files.push(FileCheckpointRecord {
                rel_path: change.rel_path.replace('\\', "/"),
                existed_before,
                before_sha256,
                existed_after,
                after_sha256,
            });
        }

        let manifest = TurnCheckpointManifest {
            workspace_id: workspace_id.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            created_at_ms,
            files,
        };

        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        write_atomic_bytes(&turn_dir.join("manifest.json"), &manifest_bytes)?;
        let _ = self.prune_workspace(workspace_id)?;
        Ok(manifest)
    }

    /// Two-phase all-or-nothing Undo:
    /// Phase 1 preflight verifies every touched file matches `after_sha256` (or absence)
    /// and has no dirty open buffer. Any conflict aborts before touching disk.
    /// Phase 2 restores prior bytes atomically (or deletes newly created files where
    /// `existed_before == false`).
    pub fn undo_turn(
        &self,
        workspace_root: &Path,
        workspace_id: &str,
        session_id: &str,
        turn_id: &str,
        state_reader: Option<&dyn WorkspaceStateReader>,
    ) -> Result<(), CheckpointError> {
        let manifest = self.load_manifest(workspace_id, session_id, turn_id)?;
        let mut conflicts = Vec::new();
        let mut plan = Vec::with_capacity(manifest.files.len());

        // Phase 1: All-or-nothing preflight
        for file in &manifest.files {
            let resolved = resolve_rooted(workspace_root, &file.rel_path)
                .map_err(|e| CheckpointError::PathEscape(e.message))?;
            let rel = rel_display(workspace_root, &resolved);

            if let Some(reader) = state_reader {
                if reader.buffer_state(&rel).is_some_and(|b| b.dirty) {
                    conflicts.push(CheckpointConflict {
                        rel_path: rel.clone(),
                        reason: "open editor buffer is dirty".to_owned(),
                    });
                    continue;
                }
            }

            let live_sha = sha256_file(&resolved)?;
            if file.existed_after {
                match (&file.after_sha256, &live_sha) {
                    (Some(expected), Some(actual)) if expected == actual => {}
                    (Some(_), None) => {
                        conflicts.push(CheckpointConflict {
                            rel_path: rel.clone(),
                            reason: "file was deleted after turn".to_owned(),
                        });
                        continue;
                    }
                    (Some(_), Some(_)) => {
                        conflicts.push(CheckpointConflict {
                            rel_path: rel.clone(),
                            reason: "file modified after turn (after_sha256 mismatch)".to_owned(),
                        });
                        continue;
                    }
                    (None, _) => {
                        conflicts.push(CheckpointConflict {
                            rel_path: rel.clone(),
                            reason: "corrupt checkpoint manifest: missing after_sha256".to_owned(),
                        });
                        continue;
                    }
                }
            } else if live_sha.is_some() {
                conflicts.push(CheckpointConflict {
                    rel_path: rel.clone(),
                    reason: "file was recreated after turn deletion".to_owned(),
                });
                continue;
            }

            let before_bytes = if file.existed_before {
                let sha = file.before_sha256.as_deref().ok_or_else(|| {
                    CheckpointError::MissingBlob {
                        sha256: "missing-before-sha256".to_owned(),
                    }
                })?;
                Some(self.load_blob(sha)?)
            } else {
                None
            };

            plan.push((resolved, before_bytes));
        }

        if !conflicts.is_empty() {
            return Err(CheckpointError::Conflict(conflicts));
        }

        // Phase 2: Atomic apply
        for (resolved, before_bytes) in plan {
            match before_bytes {
                Some(bytes) => write_atomic_bytes(&resolved, &bytes)?,
                None => match std::fs::remove_file(&resolved) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(CheckpointError::Io(e)),
                },
            }
        }

        Ok(())
    }

    /// Two-phase all-or-nothing Reapply:
    /// Phase 1 verifies live disk matches `before_sha256` (or absence if `existed_before == false`)
    /// and no dirty open buffers.
    /// Phase 2 atomically recreates/updates files from content-addressed after-blobs.
    pub fn reapply_turn(
        &self,
        workspace_root: &Path,
        workspace_id: &str,
        session_id: &str,
        turn_id: &str,
        state_reader: Option<&dyn WorkspaceStateReader>,
    ) -> Result<(), CheckpointError> {
        let manifest = self.load_manifest(workspace_id, session_id, turn_id)?;
        let mut conflicts = Vec::new();
        let mut plan = Vec::with_capacity(manifest.files.len());

        // Phase 1: Preflight
        for file in &manifest.files {
            let resolved = resolve_rooted(workspace_root, &file.rel_path)
                .map_err(|e| CheckpointError::PathEscape(e.message))?;
            let rel = rel_display(workspace_root, &resolved);

            if let Some(reader) = state_reader {
                if reader.buffer_state(&rel).is_some_and(|b| b.dirty) {
                    conflicts.push(CheckpointConflict {
                        rel_path: rel.clone(),
                        reason: "open editor buffer is dirty".to_owned(),
                    });
                    continue;
                }
            }

            let live_sha = sha256_file(&resolved)?;
            if file.existed_before {
                match (&file.before_sha256, &live_sha) {
                    (Some(expected), Some(actual)) if expected == actual => {}
                    _ => {
                        conflicts.push(CheckpointConflict {
                            rel_path: rel.clone(),
                            reason: "file diverged from before_sha256 prior to reapply".to_owned(),
                        });
                        continue;
                    }
                }
            } else if live_sha.is_some() {
                conflicts.push(CheckpointConflict {
                    rel_path: rel.clone(),
                    reason: "file already exists where existed_before == false".to_owned(),
                });
                continue;
            }

            let after_bytes = if file.existed_after {
                let sha = file.after_sha256.as_deref().ok_or_else(|| {
                    CheckpointError::MissingBlob {
                        sha256: "missing-after-sha256".to_owned(),
                    }
                })?;
                Some(self.load_blob(sha)?)
            } else {
                None
            };

            plan.push((resolved, after_bytes));
        }

        if !conflicts.is_empty() {
            return Err(CheckpointError::Conflict(conflicts));
        }

        // Phase 2: Atomic apply
        for (resolved, after_bytes) in plan {
            match after_bytes {
                Some(bytes) => write_atomic_bytes(&resolved, &bytes)?,
                None => match std::fs::remove_file(&resolved) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(CheckpointError::Io(e)),
                },
            }
        }

        Ok(())
    }

    pub fn list_workspace_turns(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<TurnCheckpointManifest>, CheckpointError> {
        validate_segment(workspace_id)?;
        let ws_dir = self.base_dir.join(workspace_id);
        let mut out = Vec::new();
        collect_manifests(&ws_dir, &mut out)?;
        out.sort_by(|a, b| {
            a.manifest
                .created_at_ms
                .cmp(&b.manifest.created_at_ms)
                .then_with(|| a.manifest.turn_id.cmp(&b.manifest.turn_id))
        });
        Ok(out.into_iter().map(|m| m.manifest).collect())
    }

    /// Enforce bounded retention:
    /// - Keep at most `max_turns_per_workspace` (default 50) newest turns for `workspace_id`.
    /// - Enforce `max_total_bytes` (default 200 MB) across the checkpoint store by pruning
    ///   oldest manifests and sweeping unreferenced blobs.
    pub fn prune_workspace(&self, workspace_id: &str) -> Result<PruneStats, CheckpointError> {
        validate_segment(workspace_id)?;
        let mut stats = PruneStats::default();

        // 1. Per-workspace turn count cap
        let ws_dir = self.base_dir.join(workspace_id);
        let mut ws_manifests = Vec::new();
        collect_manifests(&ws_dir, &mut ws_manifests)?;
        ws_manifests.sort_by(|a, b| {
            a.manifest
                .created_at_ms
                .cmp(&b.manifest.created_at_ms)
                .then_with(|| a.manifest.turn_id.cmp(&b.manifest.turn_id))
        });

        while ws_manifests.len() > self.max_turns_per_workspace {
            let oldest = ws_manifests.remove(0);
            if oldest.turn_dir.exists() {
                std::fs::remove_dir_all(&oldest.turn_dir)?;
                stats.pruned_turns += 1;
            }
        }

        // 2. Sweep unreferenced blobs
        stats.pruned_blobs += self.sweep_unreferenced_blobs()?;

        // 3. Total byte ceiling across store
        let mut total_bytes = dir_size_bytes(&self.base_dir)?;
        if total_bytes > self.max_total_bytes {
            let mut all_manifests = Vec::new();
            if self.base_dir.exists() {
                for entry in std::fs::read_dir(&self.base_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() && entry.file_name() != "blobs" {
                        collect_manifests(&path, &mut all_manifests)?;
                    }
                }
            }
            all_manifests.sort_by(|a, b| {
                a.manifest
                    .created_at_ms
                    .cmp(&b.manifest.created_at_ms)
                    .then_with(|| a.manifest.turn_id.cmp(&b.manifest.turn_id))
            });

            for item in all_manifests {
                if total_bytes <= self.max_total_bytes {
                    break;
                }
                if item.turn_dir.exists() {
                    std::fs::remove_dir_all(&item.turn_dir)?;
                    stats.pruned_turns += 1;
                }
                stats.pruned_blobs += self.sweep_unreferenced_blobs()?;
                total_bytes = dir_size_bytes(&self.base_dir)?;
            }
        }

        stats.total_bytes_remaining = total_bytes;
        Ok(stats)
    }

    fn sweep_unreferenced_blobs(&self) -> Result<usize, CheckpointError> {
        let blobs_dir = self.blobs_dir();
        if !blobs_dir.exists() {
            return Ok(0);
        }

        let mut referenced = BTreeSet::new();
        if self.base_dir.exists() {
            for entry in std::fs::read_dir(&self.base_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() && entry.file_name() != "blobs" {
                    let mut manifests = Vec::new();
                    collect_manifests(&path, &mut manifests)?;
                    for m in manifests {
                        for f in m.manifest.files {
                            if let Some(s) = f.before_sha256 {
                                referenced.insert(s);
                            }
                            if let Some(s) = f.after_sha256 {
                                referenced.insert(s);
                            }
                        }
                    }
                }
            }
        }

        let mut removed = 0;
        for entry in std::fs::read_dir(&blobs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !referenced.contains(&name) {
                    std::fs::remove_file(&path)?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}

struct StoredManifest {
    turn_dir: PathBuf,
    manifest: TurnCheckpointManifest,
}

fn collect_manifests(dir: &Path, out: &mut Vec<StoredManifest>) -> Result<(), CheckpointError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let manifest_path = path.join("manifest.json");
            if manifest_path.is_file() {
                let bytes = std::fs::read(&manifest_path)?;
                let manifest: TurnCheckpointManifest = serde_json::from_slice(&bytes)?;
                out.push(StoredManifest {
                    turn_dir: path,
                    manifest,
                });
            } else {
                collect_manifests(&path, out)?;
            }
        }
    }
    Ok(())
}

fn dir_size_bytes(dir: &Path) -> Result<u64, CheckpointError> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            total = total.saturating_add(dir_size_bytes(&path)?);
        } else if let Ok(meta) = entry.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

fn validate_segment(seg: &str) -> Result<(), CheckpointError> {
    if seg.is_empty()
        || seg == "."
        || seg == ".."
        || seg.contains('/')
        || seg.contains('\\')
        || seg.contains('\0')
        || seg.contains(':')
    {
        return Err(CheckpointError::InvalidSegment(format!(
            "unsafe checkpoint path segment `{seg}`"
        )));
    }
    Ok(())
}

fn write_atomic_bytes(target: &Path, bytes: &[u8]) -> Result<(), CheckpointError> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_sibling(target);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return Err(CheckpointError::Io(e));
    }
    drop(file);
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(CheckpointError::Io(e));
    }
    Ok(())
}

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
