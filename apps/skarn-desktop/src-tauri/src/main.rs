#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapter;
mod agents_preferences;
mod agents_preview;
mod agents_window;
mod commands;
mod crash_log;
mod events;
mod explorer;
mod hot_exit;
mod lsp_bridge;
mod security;
mod sessions;
mod state;
mod terminal;
mod updates;
mod workflow_broker;
mod workflow_command;
mod workflows_host;
mod workflows_process;
mod workspace;

use tauri::Manager;

/// Native Save / Discard / Cancel prompt for the close path. Lives in the
/// shell (not the WebView) so the prompt still appears while the editor is
/// dirty and no IPC is allowed to run.
fn prompt_close_choice(dirty: &[skarn_protocol::BufferSnapshotDto]) -> state::CloseChoice {
    const MAX_LISTED: usize = 8;
    let mut description = format!(
        "{} unsaved {}:\n\n",
        dirty.len(),
        if dirty.len() == 1 {
            "file has"
        } else {
            "files have"
        }
    );
    for buffer in dirty.iter().take(MAX_LISTED) {
        description.push_str("  • ");
        description.push_str(buffer.path.as_str());
        description.push('\n');
    }
    if dirty.len() > MAX_LISTED {
        description.push_str(&format!("  … and {} more\n", dirty.len() - MAX_LISTED));
    }
    description.push_str("\nSave all changes before closing Skarn?");

    match rfd::MessageDialog::new()
        .set_title("Unsaved changes")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            "Save".into(),
            "Discard".into(),
            "Cancel".into(),
        ))
        .show()
    {
        rfd::MessageDialogResult::Custom(label) if label == "Save" => state::CloseChoice::Save,
        rfd::MessageDialogResult::Custom(label) if label == "Discard" => {
            state::CloseChoice::Discard
        }
        _ => state::CloseChoice::Cancel,
    }
}

pub(crate) fn log_boot(msg: &str) {
    // Elapsed milliseconds since `main()` started: the perf harness reads this
    // file to split process spawn from the Rust bootstrap stages (scripts/perf.ps1).
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed_ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    let log_path = std::env::temp_dir().join("skarn_boot.log");
    let line = format!(
        "[pid {}] [+{} ms] {}\n",
        std::process::id(),
        elapsed_ms,
        msg
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

const LEGACY_WEBVIEW_APP_ID: &str = "lol.selora.ide"; // skarn-v1-migration-compat

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Atomically copy `old_dir/EBWebView` into `new_dir/EBWebView` via a `.EBWebView.migrating`
/// staging directory before `WebviewWindowBuilder` initializes WebView2.
pub(crate) fn migrate_webview2_data_dir(
    old_dir: &std::path::Path,
    new_dir: &std::path::Path,
) -> std::io::Result<bool> {
    let target = new_dir.join("EBWebView");
    let source = old_dir.join("EBWebView");
    if target.exists() || !source.is_dir() {
        return Ok(false);
    }

    let staging = new_dir.join(".EBWebView.migrating");
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }

    std::fs::create_dir_all(new_dir)?;
    if let Err(err) = copy_dir_recursive(&source, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let marker = format!(
        "{{\"migrated_from\":\"{LEGACY_WEBVIEW_APP_ID}\",\"migrated_at_unix_secs\":{secs}}}\n"
    );
    let _ = std::fs::write(new_dir.join(".migrated_from_webview"), marker);

    if let Err(err) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    Ok(true)
}

fn main() {
    // Note: Do not bind a restrictive JobObject here without breakaway flags;
    // WebView2 creates its own child process JobObjects and fails with 0x800700AA if restricted.

    // Rotating panic records live in `%APPDATA%\skarn\logs\skarn.log`.
    // The hook never touches editor buffers: saving from an unwinding thread is
    // unsafe, so it logs that fact instead (see `crash_log`).
    crash_log::install_panic_hook();

    log_boot("main() started");
    let state = state::DesktopState::new();
    log_boot("DesktopState created");
    tauri::Builder::default()
        // Must be the first plugin: a second process (for example a double-click
        // on an associated text file) hands its argv to this one and exits.
        // Without it, both processes race for the same WebView2 profile and one
        // dies during setup with a blank/frozen window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // About-dialog links leave through `FrontendCommand::OpenExternal`
        // (state.rs), which allowlists the host and calls this plugin's
        // `open_url` (ShellExecute, no shell string). The plugin's injected
        // JS click handler is off: a raw `<a target="_blank">` must stay
        // inert so every link crosses the Rust allowlist.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(state)
        .manage(agents_preferences::AgentPreferencesState::default())
        .manage(agents_window::AgentsWindowState::default())
        // §3.3 (docs/ui/PHASE_Z2_ARCHITECTURE.md): the close path runs in Rust
        // on `CloseRequested` — an IPC command cannot be guaranteed to flush
        // during webview teardown (WebView2 `beforeunload` is unreliable), so
        // the process that owns the files does the writes. Dirty buffers get a
        // native Save / Discard / Cancel prompt; cancel and failed saves call
        // `prevent_close`, so edits are never silently lost. The layout save
        // stays best-effort after the close is allowed.
        .on_window_event(|window, event| {
            agents_window::on_window_event(window, event);
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.state::<agents_window::AgentsWindowState>();
                if !lifecycle.begin_main_close() {
                    api.prevent_close();
                    return;
                }
                let app_state = window.app_handle().state::<state::DesktopState>();
                match app_state.handle_close_request(prompt_close_choice) {
                    state::CloseOutcome::Close { .. } => {
                        // Best effort: the layout must never block a close.
                        let _ = app_state.persist_layout();
                        // The app is going away: stop every ConPTY session and
                        // its process tree now rather than leaving them to the
                        // OS handle close.
                        app_state.shutdown_terminals();
                        agents_window::close_for_main_shutdown(window.app_handle());
                        window.app_handle().exit(0);
                    }
                    state::CloseOutcome::Stay => {
                        lifecycle.cancel_main_close();
                        api.prevent_close();
                    }
                    state::CloseOutcome::SaveFailed { saved, message } => {
                        lifecycle.cancel_main_close();
                        api.prevent_close();
                        let _ = rfd::MessageDialog::new()
                            .set_title("Could not save")
                            .set_description(format!(
                                "{message}\n\n{saved} buffer(s) were already saved. The window stays open."
                            ))
                            .set_buttons(rfd::MessageButtons::Ok)
                            .show();
                    }
                }
            }
        })
        .setup(|app| {
            log_boot("setup() started");
            if let Ok(new_dir) = app.path().app_local_data_dir() {
                if let Some(local_appdata) = new_dir.parent() {
                    let old_dir = local_appdata.join(LEGACY_WEBVIEW_APP_ID);
                    match migrate_webview2_data_dir(&old_dir, &new_dir) {
                        Ok(true) => log_boot("migrated WebView2 EBWebView profile"),
                        Ok(false) => {}
                        Err(err) => log_boot(&format!("WebView2 migration failed: {err}")),
                    }
                }
            }
            #[cfg(debug_assertions)]
            let webview_url = tauri::WebviewUrl::default();
            #[cfg(not(debug_assertions))]
            let webview_url = tauri::WebviewUrl::App("index.html".into());

            let window = tauri::WebviewWindowBuilder::new(app, "main", webview_url)
                .title("Skarn")
                .inner_size(1440.0, 900.0)
                .min_inner_size(960.0, 640.0)
                .center()
                // §6.3: the builder is authoritative for this manually created
                // window — tauri.conf.json mirrors it for the same reason the
                // existing code duplicates title/size in both places.
                .decorations(false)
                .visible(true)
                .skip_taskbar(false)
                .devtools(true)
                .on_navigation(security::allow_shell_navigation)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;

            log_boot("Window built successfully!");
            let _ = window.show();
            let _ = window.set_focus();
            log_boot("Window shown & focused!");

            // Phase Z3: the boot connection probe — read the keyring key,
            // probe the gateway (`list_models`), and announce the catalog
            // through `ModelsChanged` + `SkarnStatusChanged` (the agent
            // dock's model picker data path). A missing key or an
            // unreachable gateway is a silent `Disconnected` (the egui
            // boot's precedent).
            let desktop = app.state::<state::DesktopState>().inner().clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                desktop.run_boot_probe(handle).await;
            });

            // P1 hot-exit: a 2 s safety-net snapshot of dirty buffers. Edits
            // also capture immediately from the command path; this timer
            // covers a missed event and a long idle dirty state. The write
            // itself runs on the hot-exit writer thread, never on the UI.
            let hot_exit_state = app.state::<state::DesktopState>().inner().clone();
            let _ = std::thread::Builder::new()
                .name("skarn-hot-exit-timer".into())
                .spawn(move || {
                    hot_exit_state.hot_exit_prune_stale();
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        hot_exit_state.hot_exit_capture();
                    }
                });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agents_preferences::get_agent_preferences,
            agents_preferences::update_agent_preferences,
            agents_preview::agents_read_file_preview,
            agents_preview::agents_read_git_diff,
            commands::get_agents_snapshot,
            agents_window::open_agents_window,
            agents_window::agents_window_ready,
            agents_window::focus_editor_window,
            agents_window::show_session_in_main_window,
            explorer::explorer_create_entry,
            explorer::explorer_rename_entry,
            explorer::explorer_delete_entry,
            commands::get_bootstrap_state,
            commands::dispatch_command,
            commands::frontend_ready,
            commands::update_settings,
            commands::get_events_after,
            commands::connect_api_key,
            commands::disconnect_api_key,
            commands::get_api_key_status,
            commands::fetch_models,
            commands::pick_folder,
            updates::check_for_update,
            updates::install_update
        ])
        .run(tauri::generate_context!())
        .map_err(|error| {
            // No window and no IPC left to report through: write the rotating
            // log and exit non-zero instead of panicking out of `main`.
            crash_log::record_fatal(&format!("The desktop shell stopped: {error}"));
            std::process::exit(1);
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview2_migration_copies_ebwebview_atomically_and_ignores_missing_old() {
        let root = tempfile::tempdir().expect("tempdir");
        let missing_old = root.path().join("lol.selora.ide"); // skarn-v1-migration-compat
        let new_dir = root.path().join("app.skarn.desktop");
        assert!(!migrate_webview2_data_dir(&missing_old, &new_dir).expect("missing old ok"));
        assert!(!new_dir.join("EBWebView").exists());

        let old_eb = missing_old.join("EBWebView").join("Default");
        std::fs::create_dir_all(&old_eb).expect("mkdir old");
        std::fs::write(old_eb.join("Preferences"), br#"{"profile":"ok"}"#).expect("seed old");
        // Simulate stale interrupted staging directory.
        std::fs::create_dir_all(new_dir.join(".EBWebView.migrating")).expect("stale staging");
        std::fs::write(new_dir.join(".EBWebView.migrating").join("stale"), b"bad")
            .expect("stale file");

        assert!(migrate_webview2_data_dir(&missing_old, &new_dir).expect("migrate"));
        assert!(!new_dir.join(".EBWebView.migrating").exists());
        assert_eq!(
            std::fs::read_to_string(
                new_dir
                    .join("EBWebView")
                    .join("Default")
                    .join("Preferences")
            )
            .expect("copied file"),
            r#"{"profile":"ok"}"#
        );
        assert!(new_dir.join(".migrated_from_webview").is_file());

        // Existing new EBWebView is never overwritten on subsequent runs.
        std::fs::write(
            new_dir
                .join("EBWebView")
                .join("Default")
                .join("Preferences"),
            br#"{"profile":"new"}"#,
        )
        .expect("modify new");
        assert!(!migrate_webview2_data_dir(&missing_old, &new_dir).expect("second run no-op"));
        assert_eq!(
            std::fs::read_to_string(
                new_dir
                    .join("EBWebView")
                    .join("Default")
                    .join("Preferences")
            )
            .expect("new preserved"),
            r#"{"profile":"new"}"#
        );
    }
}
