//! Process-authoritative agent preferences shared by the editor and Agents window.
//! Permission grants deliberately do not belong in these persisted defaults.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Emitter;

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

fn path(_app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    selora_settings::Settings::settings_dir()
        .map(|directory| directory.join("agent-preferences.json"))
        .map_err(|error| error.to_string())
}

fn load(app: &tauri::AppHandle) -> Result<AgentPreferences, String> {
    match std::fs::read(path(app)?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Could not read agent preferences: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentPreferences::default())
        }
        Err(error) => Err(format!("Could not read agent preferences: {error}")),
    }
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
        next.selected_model = value;
    }
    if let Some(value) = patch.reasoning_effort {
        if !matches!(value.as_str(), "quick" | "standard" | "max") {
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
        *guard = Some(load(&app)?);
    }
    Ok(guard.as_ref().expect("preferences initialized").clone())
}

#[tauri::command]
pub fn update_agent_preferences(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AgentPreferencesState>,
    patch: AgentPreferencesPatch,
) -> Result<AgentPreferences, String> {
    authorize(&window)?;
    let mut guard = state.0.lock().map_err(|error| error.to_string())?;
    if guard.is_none() {
        *guard = Some(load(&app)?);
    }
    let next = patched(guard.as_ref().expect("preferences initialized"), patch)?;
    let destination = path(&app)?;
    let parent = destination.parent().ok_or("Invalid preferences path")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&next).map_err(|error| error.to_string())?;
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &destination)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Could not save agent preferences: {error}"));
    }
    *guard = Some(next.clone());
    // Preferences have their own revision domain, not the runtime replay sequence.
    let _ = app.emit("selora://agent-preferences", &next);
    Ok(next)
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
}
