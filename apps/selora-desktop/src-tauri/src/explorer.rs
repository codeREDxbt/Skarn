//! Workspace-scoped native Explorer mutations. Delete is permanent: callers must
//! show an explicit permanent-deletion warning and obtain confirmation first.

use crate::state::DesktopState;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn basename(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err("Enter a valid single file or folder name.".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    }) {
        return Err("Reserved device names are not allowed.".into());
    }
    Ok(())
}

/// Resolve every existing component, not only the final target. Refuse links
/// conservatively, including links whose destination is currently inside root.
fn existing(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    if relative.is_empty() {
        return Ok(path);
    }
    let normalized = relative.replace('\\', "/");
    for part in normalized.split('/') {
        basename(part)?;
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let mut link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            link |= metadata.file_attributes() & 0x400 != 0;
        }
        if link {
            return Err("Explorer actions cannot traverse symbolic links or junctions.".into());
        }
        let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(root) {
            return Err("The entry is outside the active workspace.".into());
        }
        path = canonical;
    }
    Ok(path)
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err("An entry with that name already exists.".into()),
    }
}

/// Keep Windows directory ancestors from being renamed into junctions during I/O.
fn pin_directories(path: &Path) -> Result<Vec<fs::File>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let mut handles = Vec::new();
        for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let handle = fs::OpenOptions::new()
                .access_mode(1) // FILE_LIST_DIRECTORY participates in sharing checks.
                .share_mode(3) // FILE_SHARE_READ | FILE_SHARE_WRITE, never FILE_SHARE_DELETE.
                .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
                .open(ancestor)
                .map_err(|error| error.to_string())?;
            if handle
                .metadata()
                .map_err(|error| error.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Explorer actions cannot traverse symbolic links or junctions.".into());
            }
            handles.push(handle);
        }
        Ok(handles)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Explorer mutations are only supported on Windows.".into())
    }
}

/// Use native exclusive rename, never std::fs::rename's replace-existing behavior.
fn rename_exclusive(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) }
            != 0
        {
            return Ok(());
        }
        Err(std::io::Error::last_os_error().to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (from, to);
        Err("Exclusive rename is not supported on this platform.".into())
    }
}

async fn offload(
    action: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn explorer_create_entry(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    workspace_id: String,
    parent_path: String,
    name: String,
    kind: String,
) -> Result<(), String> {
    basename(&name)?;
    if !matches!(kind.as_str(), "file" | "directory") {
        return Err("Unknown entry kind.".into());
    }
    let state = state.inner().clone();
    offload(move || {
        state.explorer_with_workspace(&workspace_id, |workspace| {
            let parent = existing(workspace.root(), &parent_path)?;
            let _pins = pin_directories(&parent)?;
            let parent = existing(workspace.root(), &parent_path)?;
            if !parent.is_dir() {
                return Err("The parent is not a directory.".into());
            }
            let target = parent.join(&name);
            absent(&target)?;
            match kind.as_str() {
                "file" => fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)
                    .map(|_| ()),
                _ => fs::create_dir(target),
            }
            .map_err(|e| e.to_string())
        })?;
        state.explorer_publish(&app, &workspace_id, Vec::new());
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn explorer_rename_entry(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    workspace_id: String,
    path: String,
    new_name: String,
) -> Result<(), String> {
    basename(&new_name)?;
    let state = state.inner().clone();
    offload(move || {
        let closed = state.explorer_with_workspace(&workspace_id, |workspace| {
            let source = existing(workspace.root(), &path)?;
            if source == workspace.root() {
                return Err("Cannot rename the workspace root.".into());
            }
            let parent = source.parent().ok_or("Missing parent.")?;
            let _pins = pin_directories(parent)?;
            let source = existing(workspace.root(), &path)?;
            let target = source.parent().ok_or("Missing parent.")?.join(&new_name);
            absent(&target)?;
            workspace.explorer_mutate(&[source.clone(), target.clone()], || {
                rename_exclusive(&source, &target)
            })
        })?;
        state.explorer_publish(&app, &workspace_id, closed);
        Ok(())
    })
    .await
}

/// Invoke only after the frontend confirms a permanent recursive deletion.
#[tauri::command]
pub async fn explorer_delete_entry(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    workspace_id: String,
    path: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    offload(move || {
        let closed = state.explorer_with_workspace(&workspace_id, |workspace| {
            let target = existing(workspace.root(), &path)?;
            if target == workspace.root() {
                return Err("Cannot delete the workspace root.".into());
            }
            let _pins = pin_directories(target.parent().ok_or("Missing parent.")?)?;
            let target = existing(workspace.root(), &path)?;
            workspace.explorer_mutate(std::slice::from_ref(&target), || {
                if target.is_dir() {
                    fs::remove_dir_all(&target)
                } else {
                    fs::remove_file(&target)
                }
                .map_err(|e| e.to_string())
            })
        })?;
        state.explorer_publish(&app, &workspace_id, closed);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn explorer_pins_directory_ancestors_without_blocking_child_creation() {
        let dir = tempfile::tempdir().unwrap();
        let parent = fs::canonicalize(dir.path()).unwrap();
        let pins = pin_directories(&parent).unwrap();
        fs::write(parent.join("child.txt"), "new").unwrap();
        let renamed = parent.with_file_name(format!(
            "{}-renamed",
            parent.file_name().unwrap().to_string_lossy()
        ));
        let rename = fs::rename(&parent, &renamed);
        if rename.is_ok() {
            fs::rename(&renamed, &parent).unwrap();
        }
        assert!(rename.is_err());
        drop(pins);
        assert!(parent.join("child.txt").exists());
    }

    #[test]
    fn explorer_dirty_and_clean_descendants_block_mutation() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/file.txt"), "saved").unwrap();
        fs::write(dir.path().join("unrelated.txt"), "untouched").unwrap();
        let workspace = crate::workspace::Workspace::open(dir.path().to_path_buf()).unwrap();
        let buffer = workspace.open_file("nested/file.txt").unwrap();
        let unrelated = workspace.open_file("unrelated.txt").unwrap();
        let position = selora_protocol::TextPosition {
            line: 0,
            utf16_column: 0,
        };
        workspace
            .apply_change(
                &buffer.buffer_id,
                buffer.version,
                &[selora_protocol::TextChange {
                    start: position,
                    end: position,
                    replacement: "local ".into(),
                }],
            )
            .unwrap();
        let target = workspace.root().join("nested");
        let error = workspace
            .explorer_mutate(std::slice::from_ref(&target), || {
                panic!("Dirty descendants must prevent filesystem mutation")
            })
            .unwrap_err();
        assert!(error.contains("unsaved"));
        assert!(target.exists());
        assert_eq!(workspace.buffers().len(), 2);
        workspace.save_buffer(&buffer.buffer_id).unwrap();
        let error = workspace
            .explorer_mutate(std::slice::from_ref(&target), || {
                panic!("Open clean buffers must also block mutation")
            })
            .unwrap_err();
        assert!(error.contains("open"));
        workspace.close_buffer(&buffer.buffer_id).unwrap();
        workspace
            .explorer_mutate(std::slice::from_ref(&target), || {
                fs::remove_dir_all(&target).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(workspace.save_buffer(&buffer.buffer_id).is_err());
        assert!(!target.exists());
        assert_eq!(workspace.buffers().len(), 1);
        assert_eq!(workspace.active_buffer_id(), Some(unrelated.buffer_id));
    }

    #[test]
    fn explorer_failed_mutation_keeps_clean_buffers() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "saved").unwrap();
        let workspace = crate::workspace::Workspace::open(dir.path().to_path_buf()).unwrap();
        let buffer = workspace.open_file("file.txt").unwrap();
        let result = workspace.explorer_mutate(&[workspace.root().join("file.txt")], || {
            Err("simulated failure".into())
        });
        assert!(result.is_err());
        assert_eq!(workspace.buffer(&buffer.buffer_id).unwrap().text, "saved");
        assert_eq!(workspace.active_buffer_id(), Some(buffer.buffer_id));
    }

    #[test]
    fn explorer_serializes_open_and_mutation() {
        use std::{
            sync::{mpsc, Arc},
            time::Duration,
        };
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "saved").unwrap();
        let workspace =
            Arc::new(crate::workspace::Workspace::open(dir.path().to_path_buf()).unwrap());
        let target = workspace.root().join("file.txt");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mutation_workspace = Arc::clone(&workspace);
        let mutation = std::thread::spawn(move || {
            let slice_target = target.clone();
            mutation_workspace
                .explorer_mutate(std::slice::from_ref(&slice_target), || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    fs::remove_file(target).map_err(|error| error.to_string())
                })
                .unwrap();
        });
        entered_rx.recv().unwrap();
        let (opened_tx, opened_rx) = mpsc::channel();
        let open_workspace = Arc::clone(&workspace);
        let open = std::thread::spawn(move || {
            opened_tx
                .send(open_workspace.open_file("file.txt"))
                .unwrap();
        });
        assert!(opened_rx.recv_timeout(Duration::from_millis(100)).is_err());
        release_tx.send(()).unwrap();
        mutation.join().unwrap();
        assert!(opened_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_err());
        open.join().unwrap();
        assert!(workspace.buffers().is_empty());
    }

    #[test]
    fn explorer_refuses_open_buffer_before_pending_edit_arrives() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "saved").unwrap();
        let workspace = crate::workspace::Workspace::open(dir.path().to_path_buf()).unwrap();
        let buffer = workspace.open_file("file.txt").unwrap();
        let target = workspace.root().join("file.txt");
        assert!(workspace
            .explorer_mutate(std::slice::from_ref(&target), || {
                panic!("Mutation must not run while a clean buffer is open")
            })
            .is_err());
        let pos = selora_protocol::TextPosition {
            line: 0,
            utf16_column: 0,
        };
        workspace
            .apply_change(
                &buffer.buffer_id,
                buffer.version,
                &[selora_protocol::TextChange {
                    start: pos,
                    end: pos,
                    replacement: "pending ".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            workspace.buffer(&buffer.buffer_id).unwrap().text,
            "pending saved"
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "saved");
    }

    #[cfg(windows)]
    #[test]
    fn explorer_rejects_windows_junction_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(existing(&fs::canonicalize(dir.path()).unwrap(), "junction").is_err());
        fs::remove_dir(link).unwrap();
    }

    #[test]
    fn explorer_rejects_traversal_and_invalid_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for path in [
            "../escape",
            "/absolute",
            "C:\\escape",
            "nested/../escape",
            ".",
            "a//b",
        ] {
            assert!(existing(&root, path).is_err(), "{path}");
        }
        for name in [
            "",
            "..",
            "a/b",
            "a\\b",
            "file:stream",
            "NUL.txt",
            "COM1",
            "name.",
        ] {
            assert!(basename(name).is_err(), "{name}");
        }
        assert_eq!(existing(&root, "").unwrap(), root);
        assert!(basename("file.rs").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn explorer_rename_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::write(&source, "source").unwrap();
        fs::write(&target, "target").unwrap();
        assert!(absent(&target).is_err());
        assert!(rename_exclusive(&source, &target).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "target");
        assert_eq!(fs::read_to_string(&source).unwrap(), "source");
        let renamed = dir.path().join("renamed");
        rename_exclusive(&source, &renamed).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(renamed).unwrap(), "source");
    }

    #[cfg(not(windows))]
    #[test]
    fn explorer_refuses_unpinned_mutations() {
        let dir = tempfile::tempdir().unwrap();
        assert!(pin_directories(dir.path())
            .unwrap_err()
            .contains("only supported on Windows"));
        assert!(rename_exclusive(&dir.path().join("source"), &dir.path().join("target")).is_err());
        assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn explorer_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(existing(&fs::canonicalize(dir.path()).unwrap(), "link").is_err());
    }
}
