//! Skarn Settings — persistence for user configuration.
//!
//! Responsibility (PLAN.md §5, ARCHITECTURE.md §8.1 / §10.7): the
//! `settings.json` schema, read/write at `%APPDATA%\skarn\settings.json`
//! (atomic: temp file + rename), the keybind parser the shell uses to
//! route shortcuts, and the OS-keyring get/set for the Skarn API key
//! (ARCHITECTURE.md §7 — the key's only home, never a file).
//!
//! Layout:
//! - [`settings`] — the [`Settings`] struct (ARCHITECTURE.md §8.1 schema,
//!   `_v: 1`) and `load` / `save`.
//! - [`keybind`] — [`Keybind::parse`] and the egui shortcut matcher.
//! - [`detect`] — the `which`-style PATH lookup used by the Settings dialog's
//!   "Auto-detect" buttons.
//! - [`keyring`] — the API-key get/set/delete against the Windows
//!   Credential Manager (PLAN.md §5).
//!
//! Every fallible operation returns `Result<T, SkarnError>` (PLAN.md §8);
//! version-mismatch wipes surface as [`SkarnError::Config`] so the shell can
//! toast them.

#![forbid(unsafe_code)]

mod detect;
mod keybind;
pub mod keyring;
mod settings;

pub use detect::{find_in_dirs, find_on_path};
pub use keybind::{keybind_matches, Keybind};
pub use settings::{
    env_var_with_legacy_fallback, env_var_with_legacy_source, is_known_provider,
    legacy_settings_dir_if_active, load_from_with_legacy_fallback, migrate_legacy_settings_dir,
    write_atomic, AgentSettings, EditorSettings, KeybindsSettings, LayoutSettings, LspSettings,
    Settings, SkarnSettings, UiSettings, CONFIG_DIR, EDITOR_FONT_SIZE_MAX, EDITOR_FONT_SIZE_MIN,
    EDITOR_TAB_SIZE_MAX, EDITOR_TAB_SIZE_MIN, KNOWN_PROVIDER_IDS, LEGACY_CONFIG_DIR,
    LEGACY_SETTINGS_DIR_ENV, MIGRATION_MARKER_FILE, SCHEMA_VERSION, SETTINGS_DIR_ENV,
    SETTINGS_FILE_NAME,
};

/// Re-exported so downstream shells (the Tauri desktop crate) can name the
/// error type of [`Settings::load`] / [`Settings::save`] without taking a
/// direct dependency on skarn-core.
pub use skarn_core::SkarnError;
pub use skarn_decision::ScopedGrantEntry;
