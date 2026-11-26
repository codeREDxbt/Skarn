//! Native ownership of the auxiliary Agents surface. Navigation is deliberately
//! window-local: it never changes the shared event sequence or active session.
use crate::state::DesktopState;
use skarn_protocol::ProtocolErrorDto;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager};

pub const LABEL: &str = "agents";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NavigationTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    child_id: Option<String>,
    workspace_id: Option<String>,
}

#[derive(Default)]
struct NavigationState {
    ready: bool,
    pending: Option<NavigationTarget>,
}

impl NavigationState {
    fn accept(&mut self, target: NavigationTarget) -> Option<NavigationTarget> {
        if self.ready {
            Some(target)
        } else {
            self.pending = Some(target);
            None
        }
    }

    fn mark_ready(&mut self) -> Option<NavigationTarget> {
        self.ready = true;
        self.pending.take()
    }
}

#[derive(Default)]
pub struct AgentsWindowState {
    opening: tokio::sync::Mutex<()>,
    navigation: Mutex<NavigationState>,
    geometry: Mutex<Option<Geometry>>,
    main_closing: AtomicBool,
}

impl AgentsWindowState {
    pub fn begin_main_close(&self) -> bool {
        !self.main_closing.swap(true, Ordering::SeqCst)
    }

    pub fn cancel_main_close(&self) {
        self.main_closing.store(false, Ordering::SeqCst);
    }
}

fn error(code: &str, message: impl Into<String>) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: code.into(),
        message: message.into(),
        retryable: true,
    }
}

fn known_window(label: &str) -> bool {
    matches!(label, "main" | LABEL)
}

fn authorize(window: &tauri::WebviewWindow) -> Result<(), ProtocolErrorDto> {
    if known_window(window.label()) {
        Ok(())
    } else {
        Err(error(
            "invalid_window",
            "This window cannot control Agents navigation.",
        ))
    }
}

fn validate_workspace(
    requested: Option<&str>,
    current: Option<&str>,
) -> Result<(), ProtocolErrorDto> {
    if requested.is_some_and(|id| current != Some(id)) {
        return Err(error(
            "workspace_changed",
            "The active workspace changed. Try again.",
        ));
    }
    Ok(())
}

/// Snapshot off the IPC thread: bootstrap can read the tree and Git status.
/// An explicit workspace is checked against the native workspace, never trusted
/// because the caller supplied a matching frontend label or route.
async fn navigation_target(
    desktop: &DesktopState,
    session_id: Option<String>,
    child_id: Option<String>,
    workspace_id: Option<String>,
) -> Result<NavigationTarget, ProtocolErrorDto> {
    if child_id.is_some() && session_id.is_none() {
        return Err(error(
            "invalid_navigation",
            "A child target requires a session.",
        ));
    }
    if session_id.as_ref().is_some_and(|id| id.trim().is_empty())
        || child_id.as_ref().is_some_and(|id| id.trim().is_empty())
    {
        return Err(error(
            "invalid_navigation",
            "Navigation identifiers cannot be empty.",
        ));
    }
    let desktop = desktop.clone();
    let current = tauri::async_runtime::spawn_blocking(move || {
        let workspace = desktop.bootstrap().workspace;
        // The native no-workspace sentinel is still a valid identity. Do not
        // discard it merely because no folder is open.
        Some(workspace.id.as_str().to_owned())
    })
    .await
    .map_err(|cause| error("navigation_failed", cause.to_string()))?;
    validate_workspace(workspace_id.as_deref(), current.as_deref())?;
    Ok(NavigationTarget {
        session_id,
        child_id,
        workspace_id: current,
    })
}

fn focus(window: &tauri::WebviewWindow) -> Result<(), ProtocolErrorDto> {
    window
        .unminimize()
        .and_then(|_| window.show())
        .and_then(|_| window.set_focus())
        .map_err(|cause| error("window_failed", cause.to_string()))
}

#[tauri::command]
pub async fn open_agents_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    lifecycle: tauri::State<'_, AgentsWindowState>,
    session_id: Option<String>,
    child_id: Option<String>,
    workspace_id: Option<String>,
) -> Result<(), ProtocolErrorDto> {
    authorize(&window)?;
    let target = navigation_target(&state, session_id, child_id, workspace_id).await?;
    let _opening = lifecycle.opening.lock().await;
    if lifecycle.main_closing.load(Ordering::SeqCst) {
        return Err(error("app_closing", "The editor is closing."));
    }
    let agents = match app.get_webview_window(LABEL) {
        Some(agents) => {
            let mut navigation = lifecycle
                .navigation
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(target) = navigation.accept(target) {
                app.emit_to(LABEL, "selora://agents-navigate", &target)
                    .map_err(|cause| error("navigation_failed", cause.to_string()))?;
            }
            agents
        }
        None => {
            {
                let mut navigation = lifecycle
                    .navigation
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                navigation.ready = false;
                navigation.pending = Some(target);
            }
            build_window(&app)?
        }
    };
    focus(&agents)
}

#[tauri::command]
pub fn agents_window_ready(
    window: tauri::WebviewWindow,
    lifecycle: tauri::State<'_, AgentsWindowState>,
) -> Result<Option<NavigationTarget>, ProtocolErrorDto> {
    if window.label() != LABEL {
        return Err(error(
            "invalid_window",
            "Only the Agents window can acknowledge readiness.",
        ));
    }
    let mut navigation = lifecycle
        .navigation
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    Ok(navigation.mark_ready())
}

#[tauri::command]
pub fn focus_editor_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), ProtocolErrorDto> {
    authorize(&window)?;
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| error("window_missing", "The editor window is unavailable."))?;
    focus(&main)
}

#[tauri::command]
pub async fn show_session_in_main_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<(), ProtocolErrorDto> {
    authorize(&window)?;
    let target = navigation_target(&state, Some(session_id), None, workspace_id).await?;
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| error("window_missing", "The editor window is unavailable."))?;
    app.emit_to("main", "selora://main-session-navigate", &target)
        .map_err(|cause| error("navigation_failed", cause.to_string()))?;
    focus(&main)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct Geometry {
    // Position is physical desktop space; size is logical client space.
    x: i32,
    y: i32,
    width: f64,
    height: f64,
    maximized: bool,
}

#[derive(Clone, Copy)]
struct WorkArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
}

fn clamp_geometry(saved: Option<Geometry>, area: WorkArea) -> Geometry {
    let scale = if area.scale.is_finite() && area.scale > 0.0 {
        area.scale
    } else {
        1.0
    };
    let available_width = (area.width as f64 / scale).max(1.0);
    let available_height = (area.height as f64 / scale).max(1.0);
    let valid = saved.filter(|g| g.width.is_finite() && g.height.is_finite());
    let width = valid
        .map_or(1360.0, |g| g.width)
        .max(960.0)
        .min(available_width);
    let height = valid
        .map_or(900.0, |g| g.height)
        .max(640.0)
        .min(available_height);
    let right = area.x as f64 + area.width as f64 - width * scale;
    let bottom = area.y as f64 + area.height as f64 - height * scale;
    let x = valid.map_or(
        area.x as f64 + (area.width as f64 - width * scale) / 2.0,
        |g| g.x as f64,
    );
    let y = valid.map_or(
        area.y as f64 + (area.height as f64 - height * scale) / 2.0,
        |g| g.y as f64,
    );
    Geometry {
        x: x.clamp(area.x as f64, right.max(area.x as f64)).round() as i32,
        y: y.clamp(area.y as f64, bottom.max(area.y as f64)).round() as i32,
        width,
        height,
        maximized: valid.is_some_and(|g| g.maximized),
    }
}

fn geometry_path() -> Option<std::path::PathBuf> {
    skarn_settings::Settings::settings_dir()
        .ok()
        .map(|dir| dir.join("agents-window.json"))
}

fn build_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, ProtocolErrorDto> {
    let saved = geometry_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Geometry>(&bytes).ok());
    let monitors = app
        .available_monitors()
        .map_err(|cause| error("window_failed", cause.to_string()))?;
    let monitor = saved
        .and_then(|g| {
            monitors.iter().find(|monitor| {
                let area = monitor.work_area();
                g.x as i64 >= area.position.x as i64
                    && g.y as i64 >= area.position.y as i64
                    && (g.x as i64) < area.position.x as i64 + area.size.width as i64
                    && (g.y as i64) < area.position.y as i64 + area.size.height as i64
            })
        })
        .cloned()
        .or_else(|| {
            app.get_webview_window("main")
                .and_then(|main| main.current_monitor().ok().flatten())
        })
        .or_else(|| monitors.first().cloned());
    let geometry = monitor
        .map(|monitor| {
            let area = monitor.work_area();
            clamp_geometry(
                saved,
                WorkArea {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width,
                    height: area.size.height,
                    scale: monitor.scale_factor(),
                },
            )
        })
        .unwrap_or_else(|| {
            clamp_geometry(
                saved,
                WorkArea {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    scale: 1.0,
                },
            )
        });
    *app.state::<AgentsWindowState>()
        .geometry
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(geometry);

    #[cfg(debug_assertions)]
    let url = {
        let mut url = app
            .config()
            .build
            .dev_url
            .clone()
            .ok_or_else(|| error("window_failed", "The development URL is missing."))?;
        url.query_pairs_mut().append_pair("surface", "agents");
        tauri::WebviewUrl::External(url)
    };
    #[cfg(not(debug_assertions))]
    let url = tauri::WebviewUrl::App("index.html?surface=agents".into());

    let agents = tauri::WebviewWindowBuilder::new(app, LABEL, url)
        .title("Selora Agents")
        .inner_size(geometry.width, geometry.height)
        .min_inner_size(
            960.0_f64.min(geometry.width),
            640.0_f64.min(geometry.height),
        )
        .decorations(false)
        .visible(false)
        .skip_taskbar(false)
        .devtools(cfg!(debug_assertions))
        .on_navigation(crate::security::allow_shell_navigation)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_page_load(|window, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                window
                    .state::<AgentsWindowState>()
                    .navigation
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .ready = false;
            }
        })
        .build()
        .map_err(|cause| error("window_failed", cause.to_string()))?;
    agents
        .set_position(tauri::PhysicalPosition::new(geometry.x, geometry.y))
        .map_err(|cause| error("window_failed", cause.to_string()))?;
    if geometry.maximized {
        let _ = agents.maximize();
    }
    Ok(agents)
}

/// Keep the last normal bounds while maximized/minimized. Persist only on
/// close, never on each resize and never in the editor's workspace layout.
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != LABEL {
        return;
    }
    if matches!(
        event,
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
    ) {
        capture_geometry(window);
    }
    if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
        capture_geometry(window);
        persist_geometry(window.app_handle());
        let lifecycle = window.state::<AgentsWindowState>();
        let mut navigation = lifecycle
            .navigation
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        navigation.ready = false;
        navigation.pending = None;
    }
}

fn capture_geometry(window: &tauri::Window) {
    let lifecycle = window.state::<AgentsWindowState>();
    let mut saved = lifecycle.geometry.lock().unwrap_or_else(|p| p.into_inner());
    let maximized = window.is_maximized().unwrap_or(false);
    if let Some(saved) = saved.as_mut() {
        saved.maximized = maximized;
    }
    if maximized || window.is_minimized().unwrap_or(false) {
        return;
    }
    if let (Ok(position), Ok(size), Ok(scale)) = (
        window.outer_position(),
        window.inner_size(),
        window.scale_factor(),
    ) {
        if size.width > 0 && size.height > 0 && scale.is_finite() && scale > 0.0 {
            *saved = Some(Geometry {
                x: position.x,
                y: position.y,
                width: size.width as f64 / scale,
                height: size.height as f64 / scale,
                maximized,
            });
        }
    }
}

fn persist_geometry(app: &tauri::AppHandle) {
    let geometry = *app
        .state::<AgentsWindowState>()
        .geometry
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let (Some(path), Some(geometry)) = (geometry_path(), geometry) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&geometry) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

pub fn close_for_main_shutdown(app: &tauri::AppHandle) {
    if let Some(agents) = app.get_webview_window(LABEL) {
        capture_geometry(&agents.as_ref().window());
        persist_geometry(app);
        let _ = agents.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_native_known_labels_are_allowed() {
        assert!(known_window("main"));
        assert!(known_window("agents"));
        assert!(!known_window("agents-preview"));
        assert!(!known_window("unknown"));
    }

    #[test]
    fn main_close_is_one_shot_until_cancelled() {
        let state = AgentsWindowState::default();
        assert!(state.begin_main_close());
        assert!(!state.begin_main_close());
        state.cancel_main_close();
        assert!(state.begin_main_close());
    }

    #[test]
    fn explicit_workspace_must_match_native_workspace() {
        assert!(validate_workspace(None, None).is_ok());
        assert!(validate_workspace(None, Some("current")).is_ok());
        assert!(validate_workspace(Some("current"), Some("current")).is_ok());
        assert!(validate_workspace(Some("stale"), Some("current")).is_err());
        assert!(validate_workspace(Some("current"), None).is_err());
        assert!(validate_workspace(Some(""), None).is_err());
    }

    #[test]
    fn no_workspace_navigation_uses_native_bootstrap_identity() {
        let desktop = DesktopState::with_settings_no_key(skarn_settings::Settings::default());
        let workspace = desktop.bootstrap().workspace;
        assert!(workspace.root.is_none());
        let native_id = workspace.id.as_str().to_owned();
        let target = tauri::async_runtime::block_on(navigation_target(
            &desktop,
            None,
            None,
            Some(native_id.clone()),
        ))
        .expect("opening Agents without a folder must accept the native identity");
        assert_eq!(target.workspace_id, Some(native_id.clone()));
        let target = tauri::async_runtime::block_on(navigation_target(&desktop, None, None, None))
            .expect("omitted workspace uses the native identity");
        assert_eq!(target.workspace_id, Some(native_id));
        assert!(tauri::async_runtime::block_on(navigation_target(
            &desktop,
            None,
            None,
            Some("client-invented-workspace".into()),
        ))
        .is_err());
    }

    #[test]
    fn geometry_uses_the_authoritative_settings_directory() {
        assert_eq!(
            geometry_path(),
            skarn_settings::Settings::settings_dir()
                .ok()
                .map(|directory| directory.join("agents-window.json"))
        );
    }

    #[test]
    fn readiness_drains_only_latest_pending_target_then_delivers_live() {
        let target = |session: &str| NavigationTarget {
            session_id: Some(session.into()),
            child_id: None,
            workspace_id: Some("workspace".into()),
        };
        let mut navigation = NavigationState::default();
        assert_eq!(navigation.accept(target("first")), None);
        assert_eq!(navigation.accept(target("latest")), None);
        assert_eq!(navigation.mark_ready(), Some(target("latest")));
        assert_eq!(navigation.mark_ready(), None);
        assert_eq!(navigation.accept(target("live")), Some(target("live")));
        assert_eq!(navigation.pending, None);
    }

    #[test]
    fn navigation_wire_shape_keeps_workspace_null_and_omits_empty_target() {
        let target = NavigationTarget {
            session_id: None,
            child_id: None,
            workspace_id: None,
        };
        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({"workspace_id": null})
        );
        let target = NavigationTarget {
            session_id: Some("parent".into()),
            child_id: Some("child".into()),
            workspace_id: Some("workspace".into()),
        };
        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({"session_id": "parent", "child_id": "child", "workspace_id": "workspace"})
        );
    }

    #[test]
    fn defaults_and_minimums_use_logical_pixels() {
        let area = WorkArea {
            x: -3840,
            y: 0,
            width: 3840,
            height: 2160,
            scale: 2.0,
        };
        let geometry = clamp_geometry(None, area);
        assert_eq!((geometry.width, geometry.height), (1360.0, 900.0));
        assert_eq!((geometry.x, geometry.y), (-3280, 180));
        let tiny = Geometry {
            width: 1.0,
            height: 2.0,
            ..geometry
        };
        let geometry = clamp_geometry(Some(tiny), area);
        assert_eq!((geometry.width, geometry.height), (960.0, 640.0));
    }

    #[test]
    fn removed_monitor_and_small_high_dpi_work_area_are_clamped() {
        let saved = Geometry {
            x: -9000,
            y: 9000,
            width: 2500.0,
            height: 2000.0,
            maximized: true,
        };
        let geometry = clamp_geometry(
            Some(saved),
            WorkArea {
                x: 1920,
                y: -100,
                width: 1280,
                height: 960,
                scale: 2.0,
            },
        );
        assert_eq!((geometry.x, geometry.y), (1920, -100));
        assert_eq!((geometry.width, geometry.height), (640.0, 480.0));
        assert!(geometry.maximized);
    }

    #[test]
    fn corrupt_floating_point_geometry_uses_defaults() {
        let saved = Geometry {
            x: 0,
            y: 0,
            width: f64::NAN,
            height: f64::INFINITY,
            maximized: false,
        };
        let geometry = clamp_geometry(
            Some(saved),
            WorkArea {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: f64::NAN,
            },
        );
        assert_eq!((geometry.width, geometry.height), (1360.0, 900.0));
    }
}
