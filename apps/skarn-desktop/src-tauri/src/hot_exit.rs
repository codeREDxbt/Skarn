//! Hot-exit store for unsaved editor buffers.
//!
//! This module is deliberately self-contained: it does not know about the
//! command bridge or the desktop state machine. The shell calls [`save`] on a
//! debounce (and on close), [`restore`] on boot, and [`cleanup_workspace`]
//! when a workspace is closed. Every operation is best effort: a failed
//! snapshot must never block a save or a close.
//!
//! On-disk layout: `<app-data>/hot-exit/hot-exit-<workspace-digest>.json`.
//! Each write goes to a sibling temporary file in the same directory, is
//! synced, then renamed over the previous snapshot. Nothing here writes to the
//! process temporary directory.
//!
//! Restore never touches the disk files. It returns the dirty text and lets
//! the caller decide; [`disk_changed`] tells the caller whether the disk file
//! moved on since the snapshot was taken.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use skarn_protocol::Selection;
use serde::{Deserialize, Serialize};

/// Version of the snapshot file format. A file with another version is
/// ignored by [`restore`] instead of being half-read.
pub const SCHEMA_VERSION: u32 = 1;

/// Hard cap on one serialized snapshot file (16 MiB).
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

/// Hard cap on one buffer's dirty text (4 MiB).
pub const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Hard cap on the number of buffers in one snapshot.
pub const MAX_BUFFERS: usize = 256;

const STORE_DIR_NAME: &str = "hot-exit";
const SNAPSHOT_PREFIX: &str = "hot-exit-";
const SNAPSHOT_SUFFIX: &str = ".json";
const TEMP_PREFIX: &str = ".hot-exit-";
const TEMP_SUFFIX: &str = ".tmp";

/// A hot-exit operation failed. Corrupt or foreign snapshots are not errors;
/// they are ignored by [`restore`]. This type only reports I/O and encoding
/// failures.
#[derive(Debug)]
pub enum HotExitError {
    /// Reading, creating or replacing a snapshot failed.
    Io(io::Error),
    /// A snapshot could not be serialized.
    Encode(serde_json::Error),
}

impl std::fmt::Display for HotExitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "hot-exit I/O error: {err}"),
            Self::Encode(err) => write!(formatter, "hot-exit encode error: {err}"),
        }
    }
}

impl std::error::Error for HotExitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Encode(err) => Some(err),
        }
    }
}

impl From<io::Error> for HotExitError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for HotExitError {
    fn from(err: serde_json::Error) -> Self {
        Self::Encode(err)
    }
}

/// Result type returned by this module.
pub type HotExitResult<T> = Result<T, HotExitError>;

/// One dirty buffer captured for the snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HotExitBuffer {
    /// Workspace-relative path, with either `/` or `\` separators.
    pub path: String,
    /// Buffer version at capture time.
    pub version: u64,
    /// Unsaved buffer text, exactly as the editor holds it.
    pub text: String,
    /// Cursor/selection at capture time.
    #[serde(default)]
    pub selection: Option<Selection>,
    /// Digest of the disk file when the snapshot was taken. `None` means the
    /// file could not be read, so [`disk_changed`] conservatively reports a
    /// change.
    #[serde(default)]
    pub disk_digest: Option<String>,
}

/// The input the shell supplies for one dirty buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyBuffer {
    /// Workspace-relative path.
    pub path: String,
    /// Buffer version.
    pub version: u64,
    /// Unsaved text.
    pub text: String,
    /// Cursor/selection.
    pub selection: Option<Selection>,
    /// Digest of the on-disk baseline; see [`digest_bytes`].
    pub disk_digest: Option<String>,
}

/// A buffer that was not stored, with the reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedBuffer {
    /// Workspace-relative path.
    pub path: String,
    /// Size of the skipped dirty text in bytes.
    pub bytes: usize,
    /// Stable reason code: `buffer_too_large`, `buffer_limit`, or
    /// `snapshot_too_large`.
    pub reason: &'static str,
}

/// Outcome of [`save`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SaveReport {
    /// True when a snapshot file was written (or removed because there were
    /// no dirty buffers).
    pub written: bool,
    /// Number of buffers stored.
    pub saved: usize,
    /// Serialized snapshot size in bytes.
    pub bytes: usize,
    /// Buffers left out of the snapshot.
    pub skipped: Vec<SkippedBuffer>,
}

/// Outcome of [`restore`].
#[derive(Clone, Debug)]
pub struct Restore {
    /// Workspace root recorded in the accepted snapshot.
    ///
    /// The desktop restore only consumes the buffers (it already knows the
    /// root it asked about); the field stays part of the public shape for
    /// callers that report where a snapshot came from.
    #[allow(dead_code)]
    pub workspace_root: PathBuf,
    /// Snapshot timestamp, Unix milliseconds. Same note as `workspace_root`.
    #[allow(dead_code)]
    pub saved_at_unix_ms: u64,
    /// Buffers whose disk file still exists.
    pub buffers: Vec<HotExitBuffer>,
    /// Buffers dropped because the disk file disappeared.
    pub missing_files: Vec<String>,
}

/// Outcome of [`prune_stale`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PruneReport {
    /// Snapshot and temp files removed.
    pub removed: usize,
    /// Files left in place.
    pub kept: usize,
}

/// Current time as Unix milliseconds.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Directory that holds every snapshot under the app-data directory.
pub fn store_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(STORE_DIR_NAME)
}

/// Snapshot file for one workspace.
pub fn snapshot_path(app_data_dir: &Path, workspace_root: &Path) -> PathBuf {
    let digest = digest_bytes(workspace_identity(workspace_root).as_bytes());
    store_dir(app_data_dir).join(format!("{SNAPSHOT_PREFIX}{digest}{SNAPSHOT_SUFFIX}"))
}

/// Stable content digest used for disk-change detection.
///
/// 128-bit FNV-1a: deterministic across runs and platforms, no extra crate.
/// It is not a cryptographic hash; it only detects an accidental overlay of a
/// newer disk file.
pub fn digest_bytes(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:032x}")
}

/// Save the dirty buffers for one workspace.
///
/// Empty `dirty` removes any previous snapshot: an all-clean workspace has
/// nothing to restore. Oversized buffers are skipped and reported. If nothing
/// fits, the previous snapshot is left untouched and `written` is false.
pub fn save(
    app_data_dir: &Path,
    workspace_root: &Path,
    dirty: &[DirtyBuffer],
) -> HotExitResult<SaveReport> {
    save_at(app_data_dir, workspace_root, now_unix_ms(), dirty)
}

/// [`save`] with an explicit timestamp. Tests and the close path use this.
pub fn save_at(
    app_data_dir: &Path,
    workspace_root: &Path,
    saved_at_unix_ms: u64,
    dirty: &[DirtyBuffer],
) -> HotExitResult<SaveReport> {
    let root = canonical_or_original(workspace_root);
    let path = snapshot_path(app_data_dir, &root);

    if dirty.is_empty() {
        let removed = remove_if_present(&path)?;
        return Ok(SaveReport {
            written: removed,
            saved: 0,
            bytes: 0,
            skipped: Vec::new(),
        });
    }

    let mut skipped = Vec::new();
    let mut buffers = Vec::with_capacity(dirty.len().min(MAX_BUFFERS));
    for buffer in dirty {
        if !is_safe_relative_path(&buffer.path) {
            skipped.push(SkippedBuffer {
                path: buffer.path.clone(),
                bytes: buffer.text.len(),
                reason: "invalid_path",
            });
            continue;
        }
        if buffer.text.len() > MAX_BUFFER_BYTES {
            skipped.push(SkippedBuffer {
                path: buffer.path.clone(),
                bytes: buffer.text.len(),
                reason: "buffer_too_large",
            });
            continue;
        }
        if buffers.len() >= MAX_BUFFERS {
            skipped.push(SkippedBuffer {
                path: buffer.path.clone(),
                bytes: buffer.text.len(),
                reason: "buffer_limit",
            });
            continue;
        }
        buffers.push(HotExitBuffer {
            path: buffer.path.clone(),
            version: buffer.version,
            text: buffer.text.clone(),
            selection: buffer.selection.clone(),
            disk_digest: buffer.disk_digest.clone(),
        });
    }

    if buffers.is_empty() {
        return Ok(SaveReport {
            written: false,
            saved: 0,
            bytes: 0,
            skipped,
        });
    }

    let mut snapshot = HotExitSnapshot {
        schema_version: SCHEMA_VERSION,
        workspace_root: path_to_string(&root),
        saved_at_unix_ms,
        checksum: String::new(),
        buffers,
    };

    let serialized = loop {
        snapshot.checksum = snapshot_checksum(
            snapshot.schema_version,
            &snapshot.workspace_root,
            snapshot.saved_at_unix_ms,
            &snapshot.buffers,
        )?;
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() <= MAX_SNAPSHOT_BYTES || snapshot.buffers.is_empty() {
            break bytes;
        }
        if let Some(dropped) = snapshot.buffers.pop() {
            skipped.push(SkippedBuffer {
                path: dropped.path,
                bytes: dropped.text.len(),
                reason: "snapshot_too_large",
            });
        }
    };

    if snapshot.buffers.is_empty() {
        return Ok(SaveReport {
            written: false,
            saved: 0,
            bytes: 0,
            skipped,
        });
    }

    let directory = store_dir(app_data_dir);
    fs::create_dir_all(&directory)?;
    atomic_write(&path, &serialized)?;

    Ok(SaveReport {
        written: true,
        saved: snapshot.buffers.len(),
        bytes: serialized.len(),
        skipped,
    })
}

/// Load the snapshot for `workspace_root`.
///
/// Returns `Ok(None)` when there is no snapshot, when the file is corrupt,
/// when it uses another schema version, when its checksum does not match, or
/// when it belongs to another workspace. Buffers whose disk file disappeared
/// are removed from `buffers` and listed in `missing_files`. The disk is never
/// modified.
pub fn restore(app_data_dir: &Path, workspace_root: &Path) -> HotExitResult<Option<Restore>> {
    let root = canonical_or_original(workspace_root);
    let path = snapshot_path(app_data_dir, &root);

    let bytes = match read_bounded(&path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let snapshot: HotExitSnapshot = match serde_json::from_slice(&bytes) {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok(None),
    };

    if snapshot.schema_version != SCHEMA_VERSION {
        return Ok(None);
    }

    let expected_checksum = snapshot_checksum(
        snapshot.schema_version,
        &snapshot.workspace_root,
        snapshot.saved_at_unix_ms,
        &snapshot.buffers,
    )?;
    if expected_checksum != snapshot.checksum {
        return Ok(None);
    }

    let recorded_root = PathBuf::from(&snapshot.workspace_root);
    if workspace_identity(&recorded_root) != workspace_identity(&root) {
        return Ok(None);
    }

    // A malicious or damaged snapshot must not be allowed to point outside
    // the workspace.
    if snapshot
        .buffers
        .iter()
        .any(|buffer| !is_safe_relative_path(&buffer.path))
    {
        return Ok(None);
    }

    let mut buffers = Vec::with_capacity(snapshot.buffers.len());
    let mut missing_files = Vec::new();
    for buffer in snapshot.buffers {
        if buffer_disk_path(&root, &buffer.path).is_file() {
            buffers.push(buffer);
        } else {
            missing_files.push(buffer.path);
        }
    }

    Ok(Some(Restore {
        workspace_root: recorded_root,
        saved_at_unix_ms: snapshot.saved_at_unix_ms,
        buffers,
        missing_files,
    }))
}

/// True when the disk file changed since the snapshot was taken.
///
/// A missing digest, a missing file, or an unreadable path reports `true`:
/// restore must never silently replace newer content.
pub fn disk_changed(workspace_root: &Path, buffer: &HotExitBuffer) -> io::Result<bool> {
    if !is_safe_relative_path(&buffer.path) {
        return Ok(true);
    }
    let path = buffer_disk_path(workspace_root, &buffer.path);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(err) => return Err(err),
    };
    match buffer.disk_digest.as_deref() {
        Some(expected) => Ok(digest_bytes(&bytes) != expected),
        None => Ok(true),
    }
}

/// Delete the snapshot for a closed workspace.
///
/// Returns true when a file was removed, false when there was nothing to
/// remove. This is the only operation that targets one workspace explicitly;
/// [`prune_stale`] removes snapshots whose age exceeds the caller's limit.
pub fn cleanup_workspace(app_data_dir: &Path, workspace_root: &Path) -> HotExitResult<bool> {
    let root = canonical_or_original(workspace_root);
    Ok(remove_if_present(&snapshot_path(app_data_dir, &root))?)
}

/// Remove snapshots older than `max_age` and abandoned temp files.
///
/// The age comes from the snapshot timestamp when the file parses, otherwise
/// from the file modification time. A missing store directory is not an
/// error.
pub fn prune_stale(
    app_data_dir: &Path,
    max_age: Duration,
    now_unix_ms: u64,
) -> HotExitResult<PruneReport> {
    let directory = store_dir(app_data_dir);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(PruneReport::default()),
        Err(err) => return Err(err.into()),
    };

    let age_ms = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
    let cutoff = now_unix_ms.saturating_sub(age_ms);
    let mut report = PruneReport::default();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();

        if name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX) {
            let stamp = modified_unix_ms(&path).unwrap_or(0);
            if stamp < cutoff {
                if remove_if_present(&path).unwrap_or(false) {
                    report.removed += 1;
                }
            } else {
                report.kept += 1;
            }
            continue;
        }

        if !(name.starts_with(SNAPSHOT_PREFIX) && name.ends_with(SNAPSHOT_SUFFIX)) {
            continue;
        }

        let stamp = read_snapshot(&path)
            .map(|snapshot| snapshot.saved_at_unix_ms)
            .or_else(|| modified_unix_ms(&path));

        match stamp {
            Some(stamp) if stamp < cutoff => {
                if remove_if_present(&path).unwrap_or(false) {
                    report.removed += 1;
                }
            }
            _ => report.kept += 1,
        }
    }

    Ok(report)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HotExitSnapshot {
    schema_version: u32,
    workspace_root: String,
    saved_at_unix_ms: u64,
    checksum: String,
    buffers: Vec<HotExitBuffer>,
}

fn snapshot_checksum(
    schema_version: u32,
    workspace_root: &str,
    saved_at_unix_ms: u64,
    buffers: &[HotExitBuffer],
) -> HotExitResult<String> {
    let payload = serde_json::to_vec(&(schema_version, workspace_root, saved_at_unix_ms, buffers))?;
    Ok(digest_bytes(&payload))
}

fn read_snapshot(path: &Path) -> Option<HotExitSnapshot> {
    let bytes = read_bounded(path).ok().flatten()?;
    let snapshot: HotExitSnapshot = serde_json::from_slice(&bytes).ok()?;
    (snapshot.schema_version == SCHEMA_VERSION).then_some(snapshot)
}

/// Read a snapshot only when it fits the writer's cap. A corrupt or foreign
/// file larger than the cap is ignored instead of being loaded into memory.
fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_SNAPSHOT_BYTES as u64 {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Stable workspace identity used for the file name and the root check.
/// Windows paths are case-insensitive; separators are normalized so a
/// forward-slash caller still matches the canonical form.
fn workspace_identity(path: &Path) -> String {
    let canonical = canonical_or_original(path);
    let mut identity = path_to_string(&canonical).replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    identity
}

fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.chars().any(char::is_control) {
        return false;
    }
    if value.starts_with(['/', '\\']) {
        return false;
    }
    if value.as_bytes().get(1).is_some_and(|byte| *byte == b':') {
        return false;
    }
    !value.split(['/', '\\']).any(|segment| {
        segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
    })
}

fn buffer_disk_path(workspace_root: &Path, relative: &str) -> PathBuf {
    let mut path = workspace_root.to_path_buf();
    for segment in relative
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
    {
        path.push(segment);
    }
    path
}

fn remove_if_present(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn modified_unix_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let elapsed = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

/// Same-directory temp file, `sync_all`, then rename. Never leaves a partial
/// snapshot behind and never writes outside the app-data directory.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "snapshot has no parent"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "snapshot has no name"))?;

    for _ in 0..100 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            "{TEMP_PREFIX}{}.{}-{sequence}{TEMP_SUFFIX}",
            file_name.to_string_lossy(),
            std::process::id()
        ));

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&temporary);
                    return Err(err);
                }
                drop(file);
                if let Err(err) = fs::rename(&temporary, path) {
                    let _ = fs::remove_file(&temporary);
                    return Err(err);
                }
                return Ok(());
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a hot-exit temp file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skarn_protocol::TextPosition;
    use tempfile::TempDir;

    fn position(line: u32, utf16_column: u32) -> TextPosition {
        TextPosition { line, utf16_column }
    }

    fn selection() -> Selection {
        Selection {
            anchor: position(0, 1),
            active: position(2, 3),
            version: 7,
        }
    }

    struct Fixture {
        app_data: TempDir,
        workspace: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let app_data = tempfile::tempdir().expect("app-data tempdir");
            let workspace = tempfile::tempdir().expect("workspace tempdir");
            Self {
                app_data,
                workspace,
            }
        }

        fn write_disk(&self, relative: &str, text: &str) -> PathBuf {
            let path = buffer_disk_path(self.workspace.path(), relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("disk parent");
            }
            fs::write(&path, text).expect("disk write");
            path
        }

        fn buffer(&self, relative: &str, text: &str) -> DirtyBuffer {
            let disk = self.write_disk(relative, "# baseline\n");
            let baseline = fs::read(disk).expect("baseline read");
            DirtyBuffer {
                path: relative.to_owned(),
                version: 3,
                text: text.to_owned(),
                selection: Some(selection()),
                disk_digest: Some(digest_bytes(&baseline)),
            }
        }
    }

    #[test]
    fn round_trip_preserves_text_version_and_selection() {
        let fixture = Fixture::new();
        let dirty = vec![
            fixture.buffer("src/main.rs", "fn main() {}\n"),
            fixture.buffer("README.md", "# Unsaved\n"),
        ];

        let report = save_at(
            fixture.app_data.path(),
            fixture.workspace.path(),
            1_700_000_000_000,
            &dirty,
        )
        .expect("save");
        assert!(report.written);
        assert_eq!(report.saved, 2);
        assert!(report.skipped.is_empty());

        let restore = restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .expect("snapshot exists");
        assert_eq!(restore.saved_at_unix_ms, 1_700_000_000_000);
        assert_eq!(
            restore.workspace_root,
            canonical_or_original(fixture.workspace.path())
        );
        assert!(restore.missing_files.is_empty());
        assert_eq!(restore.buffers.len(), 2);
        assert_eq!(restore.buffers[0].path, "src/main.rs");
        assert_eq!(restore.buffers[0].text, "fn main() {}\n");
        assert_eq!(restore.buffers[0].version, 3);
        assert_eq!(restore.buffers[0].selection, Some(selection()));
        assert_eq!(restore.buffers[1].text, "# Unsaved\n");
    }

    #[test]
    fn atomic_replace_keeps_only_the_new_snapshot() {
        let fixture = Fixture::new();
        let first = vec![fixture.buffer("a.txt", "first")];
        let second = vec![fixture.buffer("b.txt", "second")];

        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &first).expect("first save");
        let path = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        let first_bytes = fs::read(&path).expect("first snapshot bytes");

        save_at(
            fixture.app_data.path(),
            fixture.workspace.path(),
            2,
            &second,
        )
        .expect("second save");
        let second_bytes = fs::read(&path).expect("second snapshot bytes");
        assert_ne!(first_bytes, second_bytes);

        let restore = restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .expect("snapshot");
        assert_eq!(restore.buffers.len(), 1);
        assert_eq!(restore.buffers[0].path, "b.txt");
        assert_eq!(restore.buffers[0].text, "second");

        let leftovers: Vec<String> = fs::read_dir(store_dir(fixture.app_data.path()))
            .expect("store dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(TEMP_SUFFIX))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn oversized_buffer_is_skipped_and_reported() {
        let fixture = Fixture::new();
        let too_large = "x".repeat(MAX_BUFFER_BYTES + 1);
        let dirty = vec![
            fixture.buffer("huge.txt", &too_large),
            fixture.buffer("small.txt", "small"),
        ];

        let report =
            save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");
        assert_eq!(report.saved, 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].path, "huge.txt");
        assert_eq!(report.skipped[0].reason, "buffer_too_large");

        let restore = restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .expect("snapshot");
        assert_eq!(restore.buffers.len(), 1);
        assert_eq!(restore.buffers[0].path, "small.txt");
    }

    #[test]
    fn total_snapshot_size_is_capped() {
        let fixture = Fixture::new();
        let chunk = "y".repeat(MAX_BUFFER_BYTES);
        let dirty: Vec<DirtyBuffer> = (0..5)
            .map(|index| fixture.buffer(&format!("chunk-{index}.txt"), &chunk))
            .collect();

        let report =
            save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");
        assert!(report.written);
        assert!(
            report.bytes <= MAX_SNAPSHOT_BYTES,
            "bytes: {}",
            report.bytes
        );
        assert!(report.saved < dirty.len());
        assert!(report
            .skipped
            .iter()
            .any(|buffer| buffer.reason == "snapshot_too_large"));

        let path = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        let size = fs::metadata(&path).expect("metadata").len();
        assert!(size <= MAX_SNAPSHOT_BYTES as u64, "file size: {size}");
    }

    #[test]
    fn corrupt_snapshot_is_ignored() {
        let fixture = Fixture::new();
        let path = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        fs::create_dir_all(path.parent().expect("parent")).expect("store dir");
        fs::write(&path, b"{ this is not json").expect("corrupt write");

        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .is_none());
    }

    #[test]
    fn tampered_checksum_is_ignored() {
        let fixture = Fixture::new();
        let dirty = vec![fixture.buffer("a.txt", "original")];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");

        let path = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        value["buffers"][0]["text"] = serde_json::Value::String("tampered".into());
        fs::write(&path, serde_json::to_vec(&value).expect("serialize")).expect("write");

        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .is_none());
    }

    #[test]
    fn wrong_workspace_snapshot_is_ignored() {
        let fixture = Fixture::new();
        let other_workspace = tempfile::tempdir().expect("other workspace");
        fs::write(other_workspace.path().join("a.txt"), "disk").expect("other disk");

        let dirty = vec![fixture.buffer("a.txt", "dirty")];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");

        // Copy the file to the path the other workspace would use. The
        // recorded root inside still names the first workspace.
        let source = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        let target = snapshot_path(fixture.app_data.path(), other_workspace.path());
        fs::copy(source, target).expect("copy snapshot");

        assert!(restore(fixture.app_data.path(), other_workspace.path())
            .expect("restore")
            .is_none());
    }

    #[test]
    fn older_schema_is_ignored() {
        let fixture = Fixture::new();
        let dirty = vec![fixture.buffer("a.txt", "dirty")];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");

        let path = snapshot_path(fixture.app_data.path(), fixture.workspace.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        value["schema_version"] = serde_json::Value::from(SCHEMA_VERSION - 1);
        fs::write(&path, serde_json::to_vec(&value).expect("serialize")).expect("write");

        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .is_none());
    }

    #[test]
    fn disk_changed_detects_edit_and_disappearance() {
        let fixture = Fixture::new();
        let disk = fixture.write_disk("a.txt", "baseline");
        let buffer = HotExitBuffer {
            path: "a.txt".into(),
            version: 1,
            text: "dirty".into(),
            selection: None,
            disk_digest: Some(digest_bytes(&fs::read(&disk).expect("read"))),
        };

        assert!(!disk_changed(fixture.workspace.path(), &buffer).expect("unchanged"));

        fs::write(&disk, "changed on disk").expect("external write");
        assert!(disk_changed(fixture.workspace.path(), &buffer).expect("changed"));

        fs::remove_file(&disk).expect("remove disk");
        assert!(disk_changed(fixture.workspace.path(), &buffer).expect("missing"));

        let no_digest = HotExitBuffer {
            disk_digest: None,
            ..buffer
        };
        assert!(disk_changed(fixture.workspace.path(), &no_digest).expect("no digest"));
    }

    #[test]
    fn restore_drops_buffers_whose_disk_file_disappeared() {
        let fixture = Fixture::new();
        let dirty = vec![
            fixture.buffer("kept.txt", "kept"),
            fixture.buffer("gone.txt", "gone"),
        ];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");
        fs::remove_file(fixture.workspace.path().join("gone.txt")).expect("remove");

        let restore = restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .expect("snapshot");
        assert_eq!(restore.buffers.len(), 1);
        assert_eq!(restore.buffers[0].path, "kept.txt");
        assert_eq!(restore.missing_files, vec!["gone.txt".to_owned()]);
    }

    #[test]
    fn cleanup_workspace_removes_only_that_snapshot() {
        let fixture = Fixture::new();
        let other_workspace = tempfile::tempdir().expect("other workspace");

        let dirty = vec![fixture.buffer("a.txt", "dirty")];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");
        let other_dirty = vec![DirtyBuffer {
            path: "a.txt".into(),
            version: 1,
            text: "other".into(),
            selection: None,
            disk_digest: None,
        }];
        save_at(
            fixture.app_data.path(),
            other_workspace.path(),
            1,
            &other_dirty,
        )
        .expect("other save");

        assert!(
            cleanup_workspace(fixture.app_data.path(), fixture.workspace.path()).expect("cleanup")
        );
        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .is_none());
        assert!(
            restore(fixture.app_data.path(), other_workspace.path())
                .expect("other restore")
                .is_some(),
            "another workspace's snapshot must survive"
        );
        assert!(
            !cleanup_workspace(fixture.app_data.path(), fixture.workspace.path())
                .expect("second cleanup")
        );
    }

    #[test]
    fn prune_stale_removes_old_snapshots_and_keeps_recent_ones() {
        let fixture = Fixture::new();
        let other_workspace = tempfile::tempdir().expect("other workspace");

        let dirty = vec![fixture.buffer("a.txt", "dirty")];
        let now = 10_000_000_000u64;
        save_at(
            fixture.app_data.path(),
            fixture.workspace.path(),
            now - 2 * 60 * 60 * 1000,
            &dirty,
        )
        .expect("old save");

        let recent_dirty = vec![DirtyBuffer {
            path: "a.txt".into(),
            version: 1,
            text: "recent".into(),
            selection: None,
            disk_digest: None,
        }];
        save_at(
            fixture.app_data.path(),
            other_workspace.path(),
            now - 1000,
            &recent_dirty,
        )
        .expect("recent save");

        let report =
            prune_stale(fixture.app_data.path(), Duration::from_secs(60 * 60), now).expect("prune");
        assert_eq!(report.removed, 1);
        assert_eq!(report.kept, 1);
        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("old restore")
            .is_none());
        assert!(restore(fixture.app_data.path(), other_workspace.path())
            .expect("recent restore")
            .is_some());
    }

    #[test]
    fn saving_no_dirty_buffers_clears_the_snapshot() {
        let fixture = Fixture::new();
        let dirty = vec![fixture.buffer("a.txt", "dirty")];
        save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");

        let report =
            save_at(fixture.app_data.path(), fixture.workspace.path(), 2, &[]).expect("clear");
        assert!(report.written);
        assert_eq!(report.saved, 0);
        assert!(restore(fixture.app_data.path(), fixture.workspace.path())
            .expect("restore")
            .is_none());
    }

    #[test]
    fn unsafe_paths_are_never_written() {
        let fixture = Fixture::new();
        let dirty = vec![DirtyBuffer {
            path: "../outside.txt".into(),
            version: 1,
            text: "nope".into(),
            selection: None,
            disk_digest: None,
        }];

        let report =
            save_at(fixture.app_data.path(), fixture.workspace.path(), 1, &dirty).expect("save");
        assert!(!report.written);
        assert_eq!(report.skipped[0].reason, "invalid_path");
    }
}
