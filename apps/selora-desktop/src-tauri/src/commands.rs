use crate::state::DesktopState;
use selora_protocol::{
    ApiKeyStatusDto, BootstrapState, CommandResultDto, ConnectResultDto, ConnectionStatus,
    EventEnvelope, FrontendCommand, FrontendEvent, ModelDto, ModelsDto, ProtocolErrorDto,
    RequestEnvelope, SettingsPatchDto, PROTOCOL_VERSION,
};

/// P1 hardening: a synchronous Tauri command runs on the main/IPC thread, so
/// every blocking body (tree walks, libgit2 status, file reads/writes + fsync,
/// the `rg` subprocess, settings and keyring round-trips) froze the WebView
/// while it ran. The protocol commands are `async` now and hand that body to
/// [`tauri::async_runtime::spawn_blocking`] — the `pick_folder` pattern — so
/// the IPC thread only marshals the envelope.
///
/// `spawn_blocking` needs a `Send + 'static` closure, which is why the
/// commands clone the `DesktopState` (it is `Clone`, all fields are `Arc`s)
/// and the `AppHandle` instead of borrowing the `tauri::State` across the
/// await. A join failure becomes a typed, retryable protocol error; the wire
/// return type of every command is unchanged.
async fn offload<T, F>(work: F) -> Result<T, ProtocolErrorDto>
where
    F: FnOnce() -> Result<T, ProtocolErrorDto> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|cause| ProtocolErrorDto {
            code: "command_task_failed".into(),
            message: format!("The command task could not be joined: {cause}"),
            retryable: true,
        })?
}

/// The bootstrap body, kept out of the command so §2.3's version gate and the
/// snapshot generation can be exercised directly through [`offload`].
fn bootstrap_snapshot(
    state: &DesktopState,
    protocol_version: u32,
) -> Result<BootstrapState, ProtocolErrorDto> {
    if protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolErrorDto {
            code: "protocol_version_mismatch".into(),
            message: "Unsupported protocol version.".into(),
            retryable: false,
        });
    }
    Ok(state.bootstrap())
}

#[tauri::command]
pub async fn get_bootstrap_state(
    protocol_version: u32,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    // The bootstrap walks the tree and reads libgit2 status, so it is not the
    // cheap accessor it looks like: generate it off the IPC thread too.
    let state = state.inner().clone();
    offload(move || bootstrap_snapshot(&state, protocol_version)).await
}

#[tauri::command]
pub async fn get_agents_snapshot(
    protocol_version: u32,
    state: tauri::State<'_, DesktopState>,
) -> Result<selora_protocol::AgentsSnapshotDto, ProtocolErrorDto> {
    let state = state.inner().clone();
    offload(move || {
        if protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolErrorDto {
                code: "protocol_version_mismatch".into(),
                message: "Unsupported protocol version.".into(),
                retryable: false,
            });
        }
        Ok(state.agents_snapshot())
    })
    .await
}

/// Phase Z4 — the native folder picker for the Open Folder flow. A plain
/// Tauri command (not a protocol command): it only chooses a path; the
/// frontend then sends OpenFolderAt { root }, which is what the typed
/// boundary validates and the workspace service opens.
#[tauri::command]
pub async fn pick_folder() -> Result<Option<String>, ProtocolErrorDto> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Open Folder in Selora")
            .pick_folder()
    })
    .await
    .map_err(|cause| ProtocolErrorDto {
        code: "picker_failed".into(),
        message: format!("Folder picker failed: {cause}"),
        retryable: true,
    })?;
    Ok(picked.map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn dispatch_command(
    envelope: RequestEnvelope<FrontendCommand>,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<CommandResultDto, ProtocolErrorDto> {
    // §2.4: the app handle rides the command so accepted commands can emit
    // `WorkspaceLayoutChanged` through the existing `record_event` path.
    // `dispatch_result` is synchronous and does the command's whole body
    // (tree walks, libgit2, file I/O, the `rg` subprocess), so it runs on the
    // blocking pool. The reply is a snapshot only when the command is a read
    // (bootstrap/folder open); event-backed mutations answer a small ack.
    let state = state.inner().clone();
    offload(move || state.dispatch_result(&app, envelope)).await
}

/// Persist a settings patch through the same validated path as the protocol
/// command and publish the refreshed bootstrap to every consumer.
#[tauri::command]
pub async fn update_settings(
    patch: SettingsPatchDto,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<BootstrapState, ProtocolErrorDto> {
    // `apply_settings_update` re-reads and atomically rewrites `settings.json`
    // (temp file + fsync + rename): disk work, so never on the IPC thread.
    let state = state.inner().clone();
    offload(move || state.apply_settings_update(&app, patch, None)).await
}

/// Boot-trace marker: the frontend calls this once its bootstrap + replay flow
/// reaches `ready`. It writes one line to `%TEMP%\selora_boot.log`, so support
/// logs and `scripts/perf.ps1` can separate process/window creation from the
/// first usable shell without changing app state.
#[tauri::command]
pub fn frontend_ready() {
    crate::log_boot("frontend bootstrap ready");
}

/// §3.4's replay command — the only Tauri command Z2 adds. Backed by the
/// recorded event log and its 1024-event retention cap; the gate (seeded
/// `null`) lives on the frontend.
#[tauri::command]
pub async fn get_events_after(
    after_sequence: u64,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<EventEnvelope<FrontendEvent>>, ProtocolErrorDto> {
    // Replay clones up to 1024 envelopes under the log mutex; keep it off the
    // IPC thread so the lock is never held against the UI.
    let state = state.inner().clone();
    offload(move || Ok(state.events_after(after_sequence))).await
}

pub(crate) fn validate_provider_base_url(raw: &str) -> Result<String, ProtocolErrorDto> {
    let trimmed = raw.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(ProtocolErrorDto {
            code: "invalid_base_url".into(),
            message: "Provider Gateway URL must start with http:// or https://".into(),
            retryable: false,
        });
    }
    let parsed = url::Url::parse(trimmed).map_err(|err| ProtocolErrorDto {
        code: "invalid_base_url".into(),
        message: format!("Invalid Provider Gateway URL: {err}"),
        retryable: false,
    })?;
    if parsed.host_str().is_none() {
        return Err(ProtocolErrorDto {
            code: "invalid_base_url".into(),
            message: "Provider Gateway URL must include a valid host.".into(),
            retryable: false,
        });
    }
    Ok(trimmed.to_string())
}

pub(crate) async fn connect_api_key_inner<R: tauri::Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
    api_key: String,
    base_url: Option<String>,
    use_stored_key: bool,
) -> Result<ConnectResultDto, ProtocolErrorDto> {
    let engine = state.sessions();
    let trimmed_key = api_key.trim();
    let (candidate_key, supplied_new_key) = if !trimmed_key.is_empty() {
        (trimmed_key.to_string(), true)
    } else if use_stored_key {
        (engine.fetch_api_key()?, false)
    } else {
        return Err(ProtocolErrorDto {
            code: "invalid_api_key".into(),
            message: "API key cannot be empty.".into(),
            retryable: false,
        });
    };

    let target_base_url = match base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => validate_provider_base_url(raw)?,
        None => engine.base_url(),
    };

    // Perform real model discovery before persisting credentials or reporting Connected.
    // On failure, preserve any prior credential, catalog, and model selection.
    let client = selora_ai::SeloraClient::new(&target_base_url, &candidate_key);
    let models = match client.list_models().await {
        Ok(models) if !models.is_empty() => models,
        Ok(_) => {
            return Err(crate::sessions::empty_catalog_protocol_error(
                &target_base_url,
            ));
        }
        Err(err) => {
            return Err(crate::sessions::discovery_protocol_error(
                &target_base_url,
                &candidate_key,
                &err,
            ));
        }
    };

    let previous_key = if supplied_new_key {
        engine.key_source().get().ok().flatten()
    } else {
        None
    };

    if supplied_new_key {
        engine.key_source().set(&candidate_key)?;
    }

    let active_model = crate::sessions::reconcile_catalog_active_model(
        engine.active_model_id(),
        &models,
    );

    if let Err(err) = state.persist_selora_connection(&target_base_url, active_model.clone()) {
        if supplied_new_key {
            match previous_key.as_deref() {
                Some(old) => {
                    let _ = engine.key_source().set(old);
                }
                None => {
                    let _ = engine.key_source().delete();
                }
            }
        }
        return Err(ProtocolErrorDto {
            code: "settings_persist_failed".into(),
            message: format!(
                "Provider model discovery succeeded, but connection settings could not be saved: {err}"
            ),
            retryable: true,
        });
    }

    engine.set_base_url(target_base_url.clone());
    engine.clear_capability_cache_for(&target_base_url);
    engine.set_last_connection_error(None);
    engine.set_models_and_connection(
        models.clone(),
        ConnectionStatus::Connected,
        active_model.clone(),
    );
    state.sync_discovered_active_model(app, active_model.as_deref());

    let log = state.event_log();
    let model_dtos: Vec<ModelDto> = models.iter().map(crate::sessions::model_dto).collect();

    log.record(
        app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: model_dtos,
            active_model_id: active_model.clone(),
        }),
        None,
    );
    log.record(
        app,
        crate::sessions::status_changed_with_error(
            ConnectionStatus::Connected,
            &active_model,
            None,
        ),
        None,
    );
    let snapshot = state.bootstrap();
    log.record(
        app,
        FrontendEvent::BootstrapState(Box::new(snapshot)),
        None,
    );

    Ok(ConnectResultDto {
        connected: true,
        models_count: models.len(),
        active_model_id: active_model,
        base_url: target_base_url,
    })
}

#[tauri::command]
pub async fn connect_api_key(
    api_key: String,
    base_url: Option<String>,
    use_stored_key: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<ConnectResultDto, ProtocolErrorDto> {
    connect_api_key_inner(
        state.inner(),
        &app,
        api_key,
        base_url,
        use_stored_key.unwrap_or(false),
    )
    .await
}

pub(crate) async fn fetch_models_inner<R: tauri::Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
) -> Result<Vec<ModelDto>, ProtocolErrorDto> {
    let engine = state.sessions();
    let log = state.event_log();
    let base_url = engine.base_url();

    let api_key = match engine.fetch_api_key() {
        Ok(key) => key,
        Err(err) => {
            engine.set_connection(ConnectionStatus::Disconnected);
            engine.set_last_connection_error(Some(err.message.clone()));
            let active = engine.active_model_id();
            log.record(
                app,
                crate::sessions::status_changed_with_error(
                    ConnectionStatus::Disconnected,
                    &active,
                    Some(err.message.clone()),
                ),
                None,
            );
            let snapshot = state.bootstrap();
            log.record(
                app,
                FrontendEvent::BootstrapState(Box::new(snapshot)),
                None,
            );
            return Err(err);
        }
    };

    let client = selora_ai::SeloraClient::new(&base_url, &api_key);
    let models = match client.list_models().await {
        Ok(models) if !models.is_empty() => models,
        Ok(_) => {
            let err = crate::sessions::empty_catalog_protocol_error(&base_url);
            engine.set_connection(ConnectionStatus::Disconnected);
            engine.set_last_connection_error(Some(err.message.clone()));
            let active = engine.active_model_id();
            log.record(
                app,
                crate::sessions::status_changed_with_error(
                    ConnectionStatus::Disconnected,
                    &active,
                    Some(err.message.clone()),
                ),
                None,
            );
            let snapshot = state.bootstrap();
            log.record(
                app,
                FrontendEvent::BootstrapState(Box::new(snapshot)),
                None,
            );
            return Err(err);
        }
        Err(cause) => {
            let err = crate::sessions::discovery_protocol_error(&base_url, &api_key, &cause);
            engine.set_connection(ConnectionStatus::Disconnected);
            engine.set_last_connection_error(Some(err.message.clone()));
            let active = engine.active_model_id();
            log.record(
                app,
                crate::sessions::status_changed_with_error(
                    ConnectionStatus::Disconnected,
                    &active,
                    Some(err.message.clone()),
                ),
                None,
            );
            let snapshot = state.bootstrap();
            log.record(
                app,
                FrontendEvent::BootstrapState(Box::new(snapshot)),
                None,
            );
            return Err(err);
        }
    };

    let active_model = crate::sessions::reconcile_catalog_active_model(
        engine.active_model_id(),
        &models,
    );
    let _ = state.persist_selora_connection(&base_url, active_model.clone());

    engine.clear_capability_cache_for(&base_url);
    engine.set_last_connection_error(None);
    engine.set_models_and_connection(
        models.clone(),
        ConnectionStatus::Connected,
        active_model.clone(),
    );
    state.sync_discovered_active_model(app, active_model.as_deref());

    let model_dtos: Vec<ModelDto> = models.iter().map(crate::sessions::model_dto).collect();

    log.record(
        app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: model_dtos.clone(),
            active_model_id: active_model.clone(),
        }),
        None,
    );
    log.record(
        app,
        crate::sessions::status_changed_with_error(
            ConnectionStatus::Connected,
            &active_model,
            None,
        ),
        None,
    );
    let snapshot = state.bootstrap();
    log.record(
        app,
        FrontendEvent::BootstrapState(Box::new(snapshot)),
        None,
    );

    Ok(model_dtos)
}

#[tauri::command]
pub async fn fetch_models(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<ModelDto>, ProtocolErrorDto> {
    fetch_models_inner(state.inner(), &app).await
}

#[tauri::command]
pub async fn disconnect_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), ProtocolErrorDto> {
    let state = state.inner().clone();
    offload(move || disconnect_api_key_now(&state, &app)).await
}

/// The blocking half of [`disconnect_api_key`]: the Credential Manager delete
/// and the in-memory engine reset, run off the IPC thread.
fn disconnect_api_key_now<R: tauri::Runtime>(
    state: &DesktopState,
    app: &tauri::AppHandle<R>,
) -> Result<(), ProtocolErrorDto> {
    let engine = state.sessions();
    engine.key_source().delete()?;

    let base_url = engine.base_url();
    engine.clear_capability_cache_for(&base_url);
    engine.set_last_connection_error(None);
    engine.set_models_and_connection(Vec::new(), ConnectionStatus::Disconnected, None);
    let _ = state.set_active_model_state(app, None);
    state.sync_discovered_active_model(app, None);

    let log = state.event_log();
    log.record(
        app,
        FrontendEvent::ModelsChanged(ModelsDto {
            models: Vec::new(),
            active_model_id: None,
        }),
        None,
    );
    log.record(
        app,
        crate::sessions::status_changed_with_error(ConnectionStatus::Disconnected, &None, None),
        None,
    );

    Ok(())
}

#[tauri::command]
pub async fn get_api_key_status(
    state: tauri::State<'_, DesktopState>,
) -> Result<ApiKeyStatusDto, ProtocolErrorDto> {
    // A keyring read can take a syscall round-trip through the Credential
    // Manager; resolve the status off the IPC thread.
    let state = state.inner().clone();
    offload(move || Ok(api_key_status(&state))).await
}

pub(crate) fn api_key_status(state: &DesktopState) -> ApiKeyStatusDto {
    let engine = state.sessions();
    let base_url = engine.base_url();
    let active_model_id = engine.active_model_id();
    let connection = engine.connection();
    let last_error = engine.last_connection_error();

    let (has_key, source, key_hint) = if engine.key_source().is_test_override() {
        match engine.key_source().get() {
            Ok(Some(key)) => (true, "test_memory".to_string(), Some(mask_key(&key))),
            Ok(None) => (false, "none".to_string(), None),
            Err(_) => (false, "keyring_error".to_string(), None),
        }
    } else {
        match check_keyring() {
            (true, source, key_hint) => (true, source, key_hint),
            _ => {
                if let Ok(env_key) = std::env::var("SELORA_API_KEY") {
                    let trimmed = env_key.trim();
                    if !trimmed.is_empty() {
                        (
                            true,
                            "env: SELORA_API_KEY".to_string(),
                            Some(mask_key(trimmed)),
                        )
                    } else {
                        (false, "none".to_string(), None)
                    }
                } else if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {
                    let trimmed = env_key.trim();
                    if !trimmed.is_empty() {
                        (
                            true,
                            "env: OPENAI_API_KEY".to_string(),
                            Some(mask_key(trimmed)),
                        )
                    } else {
                        (false, "none".to_string(), None)
                    }
                } else {
                    (false, "none".to_string(), None)
                }
            }
        }
    };

    ApiKeyStatusDto {
        connected: connection == ConnectionStatus::Connected,
        has_key,
        source,
        base_url,
        key_hint,
        active_model_id,
        last_error,
    }
}

fn check_keyring() -> (bool, String, Option<String>) {
    match selora_settings::keyring::api_key_get() {
        Ok(Some(key)) => (true, "keyring".to_string(), Some(mask_key(&key))),
        Ok(None) => (false, "none".to_string(), None),
        Err(_) => (false, "keyring_error".to_string(), None),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() > 8 {
        format!("{}••••{}", &key[..4], &key[key.len().saturating_sub(4)..])
    } else {
        "••••".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use selora_protocol::{CoreInstanceId, RequestId, SurfaceId, WorkspaceId};

    /// A hermetic state: the fixed test key means no test touches the real
    /// Credential Manager (the `DesktopState` test precedent).
    fn test_state() -> DesktopState {
        DesktopState::with_settings(selora_settings::Settings::default())
    }

    fn mock_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_app().handle().clone()
    }

    fn envelope(
        state: &DesktopState,
        payload: FrontendCommand,
    ) -> RequestEnvelope<FrontendCommand> {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            workspace_id: Some(WorkspaceId::new("no-workspace").expect("valid id")),
            core_instance_id: Some(state.core_instance_id()),
            request_id: RequestId::new("request-1").expect("valid id"),
            session_id: None,
            payload,
        }
    }

    // ------------------------------------------------------------------
    // P1: the offloaded path answers exactly what the inline path did.
    // ------------------------------------------------------------------

    #[test]
    fn dispatch_offload_returns_the_same_bootstrap() {
        let state = test_state();
        let app = mock_app_handle();
        let request = envelope(&state, FrontendCommand::GetBootstrapState);

        let inline = state
            .dispatch(&app, request.clone())
            .expect("inline dispatch");
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            let app = app.clone();
            move || state.dispatch(&app, request)
        }))
        .expect("offloaded dispatch");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(offloaded.snapshot_sequence, inline.snapshot_sequence);
    }

    #[test]
    fn dispatch_offload_preserves_the_typed_error() {
        let state = test_state();
        let app = mock_app_handle();
        let mut request = envelope(&state, FrontendCommand::GetBootstrapState);
        request.core_instance_id = Some(CoreInstanceId::new("core_stale").expect("valid id"));

        let inline = state
            .dispatch(&app, request.clone())
            .expect_err("a stale core instance is rejected");
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            let app = app.clone();
            move || state.dispatch(&app, request)
        }))
        .expect_err("a stale core instance is rejected off-thread");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.code, "stale_core_instance");
    }

    #[test]
    fn bootstrap_offload_keeps_the_version_gate_and_the_snapshot() {
        let state = test_state();

        let mismatch = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || bootstrap_snapshot(&state, PROTOCOL_VERSION + 1)
        }))
        .expect_err("an unsupported protocol version is rejected");
        assert_eq!(mismatch.code, "protocol_version_mismatch");

        let snapshot = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || bootstrap_snapshot(&state, PROTOCOL_VERSION)
        }))
        .expect("bootstrap");
        assert_eq!(snapshot, state.bootstrap());
    }

    #[test]
    fn events_after_offload_replays_the_recorded_log() {
        let state = test_state();
        let app = mock_app_handle();
        // A toggle records exactly one `WorkspaceLayoutChanged`.
        state
            .dispatch(
                &app,
                envelope(
                    &state,
                    FrontendCommand::ToggleSurface {
                        surface: SurfaceId::Agent,
                    },
                ),
            )
            .expect("toggle dispatch");

        let inline = state.events_after(0);
        let offloaded = tauri::async_runtime::block_on(offload({
            let state = state.clone();
            move || Ok::<_, ProtocolErrorDto>(state.events_after(0))
        }))
        .expect("replay");

        assert_eq!(offloaded, inline);
        assert_eq!(offloaded.len(), 1);
    }

    #[tokio::test]
    async fn connect_api_key_failed_discovery_preserves_state_and_never_reports_connected() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };

        let server_fail = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&server_fail)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        let key_slot = std::sync::Arc::new(std::sync::Mutex::new(Ok(None)));
        let state = DesktopState::from_settings_path_with_mutable_key(
            settings_path,
            std::sync::Arc::clone(&key_slot),
        );
        let app = mock_app_handle();

        let bad_key = "nvapi-secret-must-not-appear-9999";
        let err = connect_api_key_inner(
            &state,
            &app,
            bad_key.into(),
            Some(format!("{}/v1", server_fail.uri())),
            false,
        )
        .await
        .expect_err("failed discovery must fail connect");
        assert_eq!(err.code, "model_discovery_failed");
        assert!(!err.message.contains(bad_key));
        assert!(!api_key_status(&state).connected);
        assert!(state.sessions().catalog_models().is_empty());
        assert!(key_slot.lock().unwrap().as_ref().unwrap().is_none());

        // Now connect successfully to server_ok with two models.
        let server_ok = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "first/model", "object": "model", "created": 1, "owned_by": "test" },
                    { "id": "mistralai/mistral-large-4-1t-a52b", "object": "model", "created": 1, "owned_by": "mistralai" }
                ]
            })))
            .mount(&server_ok)
            .await;

        let good_key = "nvapi-good-key-1234";
        let ok = connect_api_key_inner(
            &state,
            &app,
            good_key.into(),
            Some(format!("{}/v1", server_ok.uri())),
            false,
        )
        .await
        .expect("discovery succeeds");
        assert!(ok.connected);
        assert_eq!(ok.models_count, 2);
        // Initial default prefers supported model over unknown first entry.
        assert_eq!(
            ok.active_model_id.as_deref(),
            Some("mistralai/mistral-large-4-1t-a52b")
        );

        // Subsequent failing connect attempt with empty catalog must preserve prior credential & catalog.
        let server_empty = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": []
            })))
            .mount(&server_empty)
            .await;
        let second_err = connect_api_key_inner(
            &state,
            &app,
            "nvapi-second-attempt".into(),
            Some(format!("{}/v1", server_empty.uri())),
            false,
        )
        .await
        .expect_err("empty catalog fails discovery");
        assert_eq!(second_err.code, "model_discovery_failed");
        assert_eq!(
            key_slot.lock().unwrap().as_ref().unwrap().as_deref(),
            Some(good_key)
        );
        assert_eq!(state.sessions().catalog_models().len(), 2);
        assert_eq!(
            state.sessions().active_model_id().as_deref(),
            Some("mistralai/mistral-large-4-1t-a52b")
        );
    }

    #[tokio::test]
    async fn refresh_preserves_missing_selection_as_unavailable_and_boot_probe_stays_quiet() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "first/model", "object": "model", "created": 1, "owned_by": "test" },
                    { "id": "mistralai/mistral-large-4-1t-a52b", "object": "model", "created": 1, "owned_by": "mistralai" }
                ]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "first/model", "object": "model", "created": 1, "owned_by": "test" }
                ]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(503).set_body_string("gateway outage"))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        let key_slot = std::sync::Arc::new(std::sync::Mutex::new(Ok(None)));
        let state = DesktopState::from_settings_path_with_mutable_key(
            settings_path,
            key_slot,
        );
        let app = mock_app_handle();

        connect_api_key_inner(
            &state,
            &app,
            "nvapi-refresh-key".into(),
            Some(format!("{}/v1", server.uri())),
            false,
        )
        .await
        .expect("initial connect");

        // Refresh without mistral: selected model must stay identifiable, NOT silently switch to first/model.
        let refreshed = fetch_models_inner(&state, &app).await.expect("refresh succeeds");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(
            state.sessions().active_model_id().as_deref(),
            Some("mistralai/mistral-large-4-1t-a52b")
        );
        let dispatch_err = state
            .sessions()
            .resolve_model_for_dispatch(None)
            .expect_err("disappeared model must fail with unavailable");
        assert_eq!(dispatch_err.code, "model_unavailable");

        // Failed boot probe must stay Disconnected without emitting toast spam.
        let events_before = state.events_after(0).len();
        state.run_boot_probe(app.clone()).await;
        assert!(!api_key_status(&state).connected);
        assert!(api_key_status(&state).last_error.is_some());
        let new_events = state.events_after(events_before as u64);
        assert!(
            !new_events
                .iter()
                .any(|e| matches!(e.payload, FrontendEvent::Toast(_))),
            "boot probe failure must not emit toast spam"
        );
    }
}
