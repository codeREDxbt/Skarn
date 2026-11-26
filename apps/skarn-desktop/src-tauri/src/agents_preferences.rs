//! Process-authoritative agent preferences shared by the editor and Agents window.
//! Permission grants deliberately do not belong in these persisted defaults.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentPreferences {
    pub revision: u64,
    pub temperature: f64,
    pub system_instructions: String,
    pub custom_model_id: String,
    pub selected_model: String,
    pub reasoning_effort: String,
}

impl Default for AgentPreferences {
    fn default() -> Self {
        Self {
            revision: 0,
            temperature: 0.7,
            system_instructions: String::new(),
            custom_model_id: String::new(),
            selected_model: String::new(),
            reasoning_effort: "standard".into(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPreferencesPatch {
    temperature: Option<f64>,
    system_instructions: Option<String>,
    custom_model_id: Option<String>,
    selected_model: Option<String>,
    reasoning_effort: Option<String>,
}

#[derive(Default)]
pub struct AgentPreferencesState(Mutex<Option<AgentPreferences>>);

fn authorize(window: &tauri::WebviewWindow) -> Result<(), String> {
    match window.label() {
        "main" | "agents" => Ok(()),
        _ => Err("Agent preferences are not available to this window".into()),
    }
}

fn path(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    skarn_settings::Settings::settings_dir()
        .map(|directory| directory.join("agent-preferences.json"))
        .map_err(|error| error.to_string())
}

pub(crate) fn load_from_path(path: &Path) -> Result<AgentPreferences, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not read agent preferences: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentPreferences::default())
        }
        Err(error) => Err(format!("Could not read agent preferences: {error}")),
    }
}

fn load(_app: &tauri::AppHandle) -> Result<AgentPreferences, String> {
    load_from_path(&path(_app)?)
}

/// Persist preferences atomically through the shared `selora-settings` atomic writer
/// (temp file in the same directory, flush/sync, replace-safe rename).
pub(crate) fn save_to_path(path: &Path, preferences: &AgentPreferences) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(preferences).map_err(|error| error.to_string())?;
    skarn_settings::write_atomic(path, &bytes)
        .map_err(|error| format!("Could not save agent preferences: {error}"))
}

fn save(app: &tauri::AppHandle, preferences: &AgentPreferences) -> Result<(), String> {
    save_to_path(&path(app)?, preferences)
}

/// Startup reconciliation between `agent-preferences.json` and legacy `settings.selora.active_model`.
///
/// Precedence (idempotent, independent of frontend call order):
/// 1. Non-empty `agent-preferences.json` `selectedModel` wins and updates legacy `settings.selora.active_model`
///    if they differ.
/// 2. Otherwise, non-empty legacy `settings.selora.active_model` migrates into `agent-preferences.json`.
/// 3. If neither has a selection, returns `Ok(None)`.
pub(crate) fn reconcile_startup_at(
    settings_path: &Path,
    settings: &mut skarn_settings::Settings,
    preferences_path: &Path,
) -> Result<Option<String>, String> {
    let mut preferences = load_from_path(preferences_path)?;
    let pref_model = preferences.selected_model.trim().to_owned();
    let settings_model = settings
        .selora
        .active_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    if !pref_model.is_empty() {
        if settings_model.as_deref() != Some(pref_model.as_str()) {
            settings.selora.active_model = Some(pref_model.clone());
            settings
                .save_to(settings_path)
                .map_err(|error| format!("Could not migrate active model to settings: {error}"))?;
        }
        return Ok(Some(pref_model));
    }

    if let Some(legacy) = settings_model {
        preferences.selected_model = legacy.clone();
        save_to_path(preferences_path, &preferences)?;
        return Ok(Some(legacy));
    }

    Ok(None)
}

/// Persist a backend-resolved active model change (such as initial catalog default
/// or case canonicalization) into `agent-preferences.json` without overwriting
/// unrelated fields.
pub(crate) fn sync_selected_model_to_path(
    preferences_path: &Path,
    selected_model: Option<&str>,
) -> Result<(), String> {
    let mut preferences = load_from_path(preferences_path)?;
    let desired = selected_model.unwrap_or_default().trim();
    if preferences.selected_model.trim() == desired {
        return Ok(());
    }
    preferences.selected_model = desired.to_owned();
    preferences.revision = preferences.revision.saturating_add(1);
    save_to_path(preferences_path, &preferences)
}

/// Keep the in-memory Tauri `AgentPreferencesState` and frontend listeners aligned
/// when the backend updates the active model (e.g., initial catalog discovery).
pub(crate) fn sync_managed_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    selected_model: Option<&str>,
) {
    let Some(state) = app.try_state::<AgentPreferencesState>() else {
        return;
    };
    let Ok(mut guard) = state.0.lock() else {
        return;
    };
    let desired = selected_model.unwrap_or_default().trim();
    if let Some(prefs) = guard.as_mut() {
        if prefs.selected_model.trim() != desired {
            prefs.selected_model = desired.to_owned();
            prefs.revision = prefs.revision.saturating_add(1);
            let _ = app.emit("selora://agent-preferences", &*prefs);
        }
    }
}

/// Repair a split model selection left by an older build or by two windows
/// racing before the synchronization existed.
///
/// The shared preference is the user's most recent explicit selection, so it
/// wins when it differs from the persisted/live fallback. The reverse direction
/// is used only to migrate an older preference file that has no model yet.
fn reconcile_selected_model(
    app: &tauri::AppHandle,
    preferences: &mut AgentPreferences,
) -> Result<(), String> {
    let desktop = app.state::<crate::state::DesktopState>();
    let active = desktop.sessions().active_model_id();
    let requested = preferences.selected_model.trim().to_owned();

    if requested.is_empty() {
        if let Some(active) = active {
            preferences.selected_model = active;
            save(app, preferences)?;
        }
        return Ok(());
    }

    if active.as_deref() != Some(requested.as_str()) {
        let canonical = desktop
            .set_active_model(app, &requested)
            .map_err(|error| error.message)?;
        if preferences.selected_model != canonical {
            preferences.selected_model = canonical;
            save(app, preferences)?;
        }
    }
    Ok(())
}

fn patched(
    current: &AgentPreferences,
    patch: AgentPreferencesPatch,
) -> Result<AgentPreferences, String> {
    let mut next = current.clone();
    if let Some(value) = patch.temperature {
        if !value.is_finite() || !(0.0..=2.0).contains(&value) {
            return Err("Temperature must be between 0 and 2".into());
        }
        next.temperature = value;
    }
    if let Some(value) = patch.system_instructions {
        if value.len() > 65_536 {
            return Err("System instructions exceed 64 KiB".into());
        }
        next.system_instructions = value;
    }
    if let Some(value) = patch.custom_model_id {
        if value.len() > 512 {
            return Err("Custom model ID is too long".into());
        }
        next.custom_model_id = value;
    }
    if let Some(value) = patch.selected_model {
        if value.len() > 512 {
            return Err("Selected model ID is too long".into());
        }
        next.selected_model = value.trim().to_owned();
    }
    if let Some(value) = patch.reasoning_effort {
        if !matches!(value.as_str(), "none" | "quick" | "standard" | "max") {
            return Err("Unknown reasoning effort".into());
        }
        next.reasoning_effort = value;
    }
    next.revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| "Agent preferences revision overflow".to_owned())?;
    Ok(next)
}

#[tauri::command]
pub fn get_agent_preferences(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentPreferencesState>,
) -> Result<AgentPreferences, String> {
    authorize(&window)?;
    let mut guard = state.0.lock().map_err(|error| error.to_string())?;
    if guard.is_none() {
        let mut preferences = load(&app)?;
        reconcile_selected_model(&app, &mut preferences)?;
        *guard = Some(preferences);
    }
    Ok(guard.as_ref().expect("preferences initialized").clone())
}

pub(crate) fn update_preferences_with_saver<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    desktop: &crate::state::DesktopState,
    state: &AgentPreferencesState,
    prefs_path: &Path,
    patch: AgentPreferencesPatch,
    save_fn: impl FnOnce(&Path, &AgentPreferences) -> Result<(), String>,
) -> Result<AgentPreferences, String> {
    let mut guard = state.0.lock().map_err(|error| error.to_string())?;
    if guard.is_none() {
        *guard = Some(load_from_path(prefs_path)?);
    }
    let current = guard.as_ref().expect("preferences initialized").clone();
    let selects_model = patch.selected_model.is_some();
    let mut next = patched(&current, patch)?;

    let previous_active = if selects_model {
        let previous = desktop.sessions().active_model_id();
        let requested = next.selected_model.trim();
        let desired = (!requested.is_empty()).then_some(requested);
        let canonical = desktop
            .set_active_model_state(app, desired)
            .map_err(|error| error.message)?;
        next.selected_model = canonical.unwrap_or_default();
        Some(previous)
    } else {
        None
    };

    if let Err(error) = save_fn(prefs_path, &next) {
        if let Some(previous) = previous_active {
            // The preference write is the second half of one logical change.
            // Restore the previous live/persisted model so the next request
            // cannot observe a half-applied selection.
            if let Err(rollback) = desktop.set_active_model_state(app, previous.as_deref()) {
                return Err(format!(
                    "{error}; the previous model could not be restored: {}",
                    rollback.message
                ));
            }
        }
        return Err(error);
    }

    *guard = Some(next.clone());
    // Preferences have their own revision domain, not the runtime replay sequence.
    let _ = app.emit("selora://agent-preferences", &next);
    Ok(next)
}

#[tauri::command]
pub fn update_agent_preferences(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentPreferencesState>,
    patch: AgentPreferencesPatch,
) -> Result<AgentPreferences, String> {
    authorize(&window)?;
    let prefs_path = path(&app)?;
    let desktop = app.state::<crate::state::DesktopState>();
    update_preferences_with_saver(&app, &desktop, &state, &prefs_path, patch, save_to_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_patches_preserve_other_window_changes() {
        let first = patched(
            &AgentPreferences::default(),
            AgentPreferencesPatch {
                system_instructions: Some("Keep existing comments.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let second = patched(
            &first,
            AgentPreferencesPatch {
                reasoning_effort: Some("max".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(second.system_instructions, "Keep existing comments.");
        assert_eq!(second.reasoning_effort, "max");
        assert_eq!(second.revision, 2);
    }

    #[test]
    fn invalid_patch_does_not_change_current_preferences() {
        let current = AgentPreferences::default();
        assert!(patched(
            &current,
            AgentPreferencesPatch {
                temperature: Some(f64::NAN),
                ..Default::default()
            }
        )
        .is_err());
        assert_eq!(current.revision, 0);
        assert_eq!(current.temperature, 0.7);
    }

    #[test]
    fn wire_uses_frontend_field_names_and_excludes_permission_grants() {
        let value = serde_json::to_value(AgentPreferences::default()).unwrap();
        assert_eq!(value["reasoningEffort"], "standard");
        assert!(value.get("permissionMode").is_none());
        assert!(
            serde_json::from_value::<AgentPreferencesPatch>(serde_json::json!({
                "permissionMode": "full"
            }))
            .is_err()
        );
    }

    #[test]
    fn repeated_preference_saves_succeed_and_failure_rolls_back_model() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        let prefs_path = dir.path().join("agent-preferences.json");

        // Repeated atomic saves replace the existing file cleanly on Windows.
        for idx in 0..5 {
            let prefs = AgentPreferences {
                selected_model: format!("provider/model-{idx}"),
                revision: idx as u64,
                ..Default::default()
            };
            save_to_path(&prefs_path, &prefs).expect("repeated atomic preference save");
        }
        assert_eq!(
            load_from_path(&prefs_path).expect("load saved prefs").selected_model,
            "provider/model-4"
        );

        let desktop = crate::state::DesktopState::from_settings_path(settings_path.clone());
        let app = tauri::test::mock_app();
        let handle = app.handle();
        desktop
            .set_active_model(handle, "provider/initial-model")
            .expect("set initial model");
        let pref_state = AgentPreferencesState::default();

        let err = update_preferences_with_saver(
            handle,
            &desktop,
            &pref_state,
            &prefs_path,
            AgentPreferencesPatch {
                selected_model: Some("provider/failed-model".into()),
                ..Default::default()
            },
            |_path, _prefs| Err("simulated disk full".into()),
        )
        .expect_err("failed preference write must error");
        assert!(err.contains("simulated disk full"));
        assert_eq!(
            desktop.sessions().active_model_id().as_deref(),
            Some("provider/initial-model")
        );
        let persisted = skarn_settings::Settings::load_from(&settings_path).expect("settings load");
        assert_eq!(
            persisted.selora.active_model.as_deref(),
            Some("provider/initial-model")
        );
    }

    #[test]
    fn startup_reconciliation_syncs_preferences_and_legacy_settings_idempotently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        let prefs_path = dir.path().join("agent-preferences.json");

        // Case 1: legacy settings.selora.active_model migrates into agent-preferences.json.
        let mut legacy = skarn_settings::Settings::default();
        legacy.selora.active_model = Some("legacy/active-model".into());
        legacy.save_to(&settings_path).expect("save legacy settings");

        let reconciled =
            reconcile_startup_at(&settings_path, &mut legacy, &prefs_path).expect("reconcile 1");
        assert_eq!(reconciled.as_deref(), Some("legacy/active-model"));
        assert_eq!(
            load_from_path(&prefs_path).unwrap().selected_model,
            "legacy/active-model"
        );

        // Case 2: non-empty agent-preferences.json selectedModel overrides differing settings.json.
        save_to_path(
            &prefs_path,
            &AgentPreferences {
                selected_model: "prefs/authoritative-model".into(),
                ..Default::default()
            },
        )
        .expect("save prefs");
        let mut loaded = skarn_settings::Settings::load_from(&settings_path).unwrap();
        let reconciled_2 =
            reconcile_startup_at(&settings_path, &mut loaded, &prefs_path).expect("reconcile 2");
        assert_eq!(reconciled_2.as_deref(), Some("prefs/authoritative-model"));
        assert_eq!(
            skarn_settings::Settings::load_from(&settings_path)
                .unwrap()
                .selora
                .active_model
                .as_deref(),
            Some("prefs/authoritative-model")
        );
    }
}
