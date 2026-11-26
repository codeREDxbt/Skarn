//! In-app updates (UI_SPEC.md §11, RELEASE.md §4).
//!
//! The release pipeline publishes `Selora-IDE-Setup-<version>.exe` and its
//! `.sha256` on every GitHub release. This module checks for a newer tag,
//! downloads the installer, verifies the published digest and Authenticode
//! signature, and hands the file to Windows. The installer's `AppId` is stable,
//! so it upgrades the existing per-user install in place: no settings loss.
//!
//! Nothing is downloaded until the user answers the startup prompt (or presses
//! Install in the About dialog); the check itself is one GitHub API GET.

use selora_protocol::ProtocolErrorDto;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Emitter;

/// Progress event the UpdateDialog listens for.
pub const UPDATE_PROGRESS_EVENT: &str = "selora:update-progress";

/// Stage 0 kill-switch: mirrors [`selora_ai::updates::UPDATES_ENABLED`] so
/// `check_for_update` and `install_update` make zero network calls until
/// `codeREDxbt/Skarn` has live signed releases.
pub const UPDATES_ENABLED: bool = selora_ai::updates::UPDATES_ENABLED;

/// The running build's version, from the workspace manifest.
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckDto {
    pub current_version: String,
    pub latest_tag: String,
    pub newer: bool,
    /// The release page, used when the in-app installer is unavailable.
    pub release_url: String,
    /// True when both the setup and its checksum are attached to the release.
    pub installer_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgressDto {
    pub downloaded: u64,
    /// 0 when the server did not report a content length.
    pub total: u64,
}

fn update_error(message: impl Into<String>, retryable: bool) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "update_failed".into(),
        message: message.into(),
        retryable,
    }
}

fn updates_disabled_error() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "updates_disabled".into(),
        message: "In-app updates are disabled in this build (release channel not configured yet)."
            .into(),
        retryable: false,
    }
}

fn map_net(cause: selora_core::SeloraError) -> ProtocolErrorDto {
    update_error(format!("Update request failed: {cause}"), true)
}

/// Where a downloaded installer is staged. Per-user, beside the app data the
/// installer already owns; falls back to the system temp directory.
fn update_dir() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(root) => PathBuf::from(root).join("Skarn").join("updates"),
        None => std::env::temp_dir().join("skarn-updates"),
    }
}

/// One GET against the release endpoint. Never downloads, never installs.
#[tauri::command]
pub async fn check_for_update() -> Result<UpdateCheckDto, ProtocolErrorDto> {
    if !UPDATES_ENABLED {
        return Err(updates_disabled_error());
    }
    let release = selora_ai::updates::check_latest_release(CURRENT_VERSION)
        .await
        .map_err(map_net)?;
    Ok(UpdateCheckDto {
        current_version: CURRENT_VERSION.to_string(),
        latest_tag: release.tag,
        newer: release.newer,
        release_url: release.html_url,
        installer_available: release.installer.is_some() && release.checksum.is_some(),
    })
}

/// Download the latest installer, verify it, launch it silently, and exit so
/// the installer can replace the running executable.
///
/// The caller is the user's explicit confirmation, and only that: a check
/// that found nothing newer answers a typed error instead of downloading.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), ProtocolErrorDto> {
    install_update_inner(&app).await
}

async fn install_update_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), ProtocolErrorDto> {
    if !UPDATES_ENABLED {
        return Err(updates_disabled_error());
    }
    let release = selora_ai::updates::check_latest_release(CURRENT_VERSION)
        .await
        .map_err(map_net)?;
    if !release.newer {
        return Err(update_error(
            format!("Skarn {} is already the latest release.", CURRENT_VERSION),
            false,
        ));
    }
    let installer = release
        .installer
        .ok_or_else(|| update_error("This release has no Windows installer attached.", false))?;
    let checksum = release.checksum.ok_or_else(|| {
        update_error(
            "This release has no published SHA256 checksum; refusing to install it.",
            false,
        )
    })?;

    let expected = selora_ai::updates::fetch_checksum(&checksum.download_url)
        .await
        .map_err(map_net)?;

    // Throttle IPC: one event per MiB (plus the final byte).
    let last_emitted = AtomicU64::new(0);
    let emitter = app.clone();
    let progress = move |downloaded: u64, total: u64| {
        let previous = last_emitted.load(Ordering::Relaxed);
        if downloaded != total && downloaded.saturating_sub(previous) < 1_000_000 {
            return;
        }
        last_emitted.store(downloaded, Ordering::Relaxed);
        let _ = emitter.emit(
            UPDATE_PROGRESS_EVENT,
            UpdateProgressDto { downloaded, total },
        );
    };

    let setup =
        selora_ai::updates::download_installer(&installer, &expected, &update_dir(), &progress)
            .await
            .map_err(map_net)?;

    if let Err(error) = verify_authenticode(&setup) {
        let _ = std::fs::remove_file(&setup);
        return Err(error);
    }
    launch_installer(&setup)?;
    // The installer closes/replaces this executable; exit before it tries.
    app.exit(0);
    Ok(())
}

#[cfg(windows)]
fn verify_authenticode(setup: &std::path::Path) -> Result<(), ProtocolErrorDto> {
    // Pass the path through the environment, not PowerShell command text.
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "if ((Get-AuthenticodeSignature -LiteralPath $env:SKARN_UPDATE_INSTALLER).Status -ne 'Valid') { exit 1 }",
        ])
        .env("SKARN_UPDATE_INSTALLER", setup)
        .output()
        .map_err(|error| update_error(format!("Could not verify installer signature: {error}"), false))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(update_error(
            "Installer Authenticode signature is not valid; refusing to install it.",
            false,
        ))
    }
}

#[cfg(not(windows))]
fn verify_authenticode(_: &std::path::Path) -> Result<(), ProtocolErrorDto> {
    Err(update_error(
        "Automatic installer updates are supported on Windows only.",
        false,
    ))
}

/// Spawn `cmd /c start /wait <setup> /SILENT ... && start <exe>` detached.
///
/// `/CLOSEAPPLICATIONS` lets Inno Setup close a still-running instance;
/// `/NORESTART` suppresses a reboot prompt. The chained `start` reopens
/// Selora IDE once the installer exits, which `/SILENT` would otherwise skip
/// (`[Run]` is `skipifsilent`).
fn launch_installer(setup: &std::path::Path) -> Result<(), ProtocolErrorDto> {
    let exe = std::env::current_exe()
        .map_err(|error| update_error(format!("Cannot resolve the running app: {error}"), true))?;
    let script = format!(
        "start \"\" /wait \"{}\" /SILENT /CLOSEAPPLICATIONS /NORESTART && start \"\" \"{}\"",
        setup.display(),
        exe.display()
    );

    let mut command = std::process::Command::new("cmd");
    command
        .args(["/c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flash behind the WebView.
        command.creation_flags(0x0800_0000);
    }

    command.spawn().map_err(|error| {
        update_error(
            format!("Could not start the downloaded installer: {error}"),
            true,
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_version_is_the_workspace_triple() {
        // `scripts/check-release-metadata.ps1` keeps Cargo.toml, tauri.conf.json,
        // and package.json equal; this is the version the prompt compares.
        assert!(
            selora_ai::updates::parse_checksum(&"0".repeat(64)).is_some(),
            "sanity: a 64-hex digest still parses"
        );
        let parts: Vec<&str> = CURRENT_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "version is a numeric triple");
        assert!(parts.iter().all(|part| part.parse::<u64>().is_ok()));
    }

    #[test]
    fn the_update_staging_dir_is_per_user() {
        let dir = update_dir();
        assert!(dir.ends_with("updates") || dir.ends_with("skarn-updates"));
    }

    #[tokio::test]
    async fn check_and_install_update_are_disabled_with_zero_network_calls() {
        assert!(!UPDATES_ENABLED);
        let check_err = check_for_update()
            .await
            .expect_err("check_for_update must return disabled status");
        assert_eq!(check_err.code, "updates_disabled");
        assert!(!check_err.retryable);

        let app = tauri::test::mock_app();
        let install_err = install_update_inner(app.handle())
            .await
            .expect_err("install_update must return disabled status");
        assert_eq!(install_err.code, "updates_disabled");
        assert!(!install_err.retryable);
    }

    #[cfg(windows)]
    #[test]
    fn unsigned_or_missing_installer_is_rejected() {
        let missing = update_dir().join("selora-nonexistent-signature-test.exe");
        assert!(verify_authenticode(&missing).is_err());
        let unsigned =
            std::env::temp_dir().join(format!("selora-unsigned-{}.exe", std::process::id()));
        std::fs::write(&unsigned, b"unsigned installer").unwrap();
        let result = verify_authenticode(&unsigned);
        std::fs::remove_file(&unsigned).unwrap();
        assert!(result.is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn installer_verification_is_windows_only() {
        assert!(verify_authenticode(std::path::Path::new("installer.exe")).is_err());
    }
}
