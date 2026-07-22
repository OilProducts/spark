#![forbid(unsafe_code)]

//! Axum HTTP composition for Spark Workspace compatibility routes.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use serde_json::json;
use serde_json::Value;
use spark_agent_adapter::{AgentTurnBackend, RustLlmAgentTurnBackend};
use spark_common::settings::SparkSettings;
use spark_workspace::live::{
    full_run_usage_accumulator, latest_run_sequence, run_live_publication,
    run_upsert_envelope_with_usage, trigger_upsert_envelope, LiveEnvelope, RunUsageAccumulator,
};
use spark_workspace::{WorkspaceError, WorkspaceTriggerService};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;

mod workspace;

pub fn build_app(settings: SparkSettings) -> Router {
    let agent_turn_backend =
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::default_runtime_handler_runner_factory(),
        agent_turn_backend,
    )
}

pub fn build_app_with_rust_llm_client(
    settings: SparkSettings,
    client: unified_llm_adapter::Client,
) -> Router {
    let agent_turn_backend: Arc<dyn AgentTurnBackend> =
        Arc::new(RustLlmAgentTurnBackend::new(client.clone()));
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::rust_llm_runtime_handler_runner_factory(client),
        agent_turn_backend,
    )
}

pub fn build_app_with_agent_turn_backend(
    settings: SparkSettings,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Router {
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        attractor_api::default_runtime_handler_runner_factory(),
        agent_turn_backend,
    )
}

pub fn build_app_with_runtime_handler_runner_factory(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
) -> Router {
    let agent_turn_backend =
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
    build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings,
        runtime_handler_runner_factory,
        agent_turn_backend,
    )
}

pub fn build_app_with_runtime_handler_runner_factory_and_agent_turn_backend(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
) -> Router {
    let settings = Arc::new(settings);
    let live_hub = Arc::new(WorkspaceLiveHub::new());
    let (run_event_observer, run_event_publisher) =
        RunEventPublisher::spawn(settings.clone(), live_hub.clone());
    let state = HttpAppState {
        settings,
        live_hub,
        runtime_handler_runner_factory,
        agent_turn_backend,
        run_event_observer,
        trigger_source_loop: None,
        run_event_publisher,
    };
    let state = state.with_trigger_source_loop();
    // Runs execute as threads, so any previous process's restart orphaned its
    // non-terminal runs; re-arm them before serving traffic. Resumption is
    // detached — this does not block startup.
    {
        let mut service =
            attractor_api::AttractorApiService::new_with_runtime_handler_runner_factory(
                (*state.settings).clone(),
                state.runtime_handler_runner_factory.clone(),
            );
        if let Some(observer) = state.run_event_observer.0.clone() {
            service = service.with_run_event_observer(observer);
        }
        let _ = service.recover_interrupted_runs();
    }
    Router::new()
        .route("/", get(serve_index))
        .route("/favicon.ico", get(serve_favicon))
        .route("/assets/{*asset_path}", get(serve_asset))
        .route("/workspace", any(redirect_workspace_mount))
        .nest(
            "/workspace/api",
            workspace::router().fallback(workspace_api_fallback),
        )
        .route("/attractor", any(redirect_attractor_mount))
        .route("/attractor/{*path}", any(attractor_dispatch))
        .fallback(product_fallback)
        .with_state(state)
}

#[derive(Clone)]
pub(crate) struct HttpAppState {
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
    run_event_observer: RunEventObserverHandle,
    #[allow(dead_code)]
    trigger_source_loop: Option<Arc<TriggerSourceLoop>>,
    #[allow(dead_code)]
    run_event_publisher: Option<Arc<RunEventPublisher>>,
}

/// The app-wide run-event observer handed to every execution surface; fires
/// into the coalescing publisher so background runs stream live updates.
#[derive(Clone, Default)]
pub(crate) struct RunEventObserverHandle(pub(crate) Option<attractor_api::RunEventObserver>);

impl FromRef<HttpAppState> for RunEventObserverHandle {
    fn from_ref(input: &HttpAppState) -> Self {
        input.run_event_observer.clone()
    }
}

impl FromRef<HttpAppState> for Arc<SparkSettings> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.settings.clone()
    }
}

impl FromRef<HttpAppState> for Arc<WorkspaceLiveHub> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.live_hub.clone()
    }
}

impl FromRef<HttpAppState> for attractor_api::RuntimeHandlerRunnerFactory {
    fn from_ref(input: &HttpAppState) -> Self {
        input.runtime_handler_runner_factory.clone()
    }
}

impl FromRef<HttpAppState> for Arc<dyn AgentTurnBackend> {
    fn from_ref(input: &HttpAppState) -> Self {
        input.agent_turn_backend.clone()
    }
}

#[derive(Debug)]
pub(crate) struct WorkspaceLiveHub {
    sender: broadcast::Sender<LiveEnvelope>,
}

impl WorkspaceLiveHub {
    fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(256);
        Self { sender }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<LiveEnvelope> {
        self.sender.subscribe()
    }

    pub(crate) fn publish(&self, envelope: LiveEnvelope) {
        let _ = self.sender.send(envelope);
    }
}

struct RunEventPublisher {
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

impl RunEventPublisher {
    /// Spawns the coalescing publisher and returns the observer that feeds
    /// it. Without a tokio runtime (pure-sync callers) both are inert.
    fn spawn(
        settings: Arc<SparkSettings>,
        live_hub: Arc<WorkspaceLiveHub>,
    ) -> (RunEventObserverHandle, Option<Arc<Self>>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return (RunEventObserverHandle(None), None);
        };
        let (sender, receiver) = mpsc::unbounded_channel::<String>();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_event_publisher_loop(
            settings,
            live_hub,
            receiver,
            task_cancellation,
        ));
        let observer: attractor_api::RunEventObserver = Arc::new(move |run_id: &str| {
            let _ = sender.send(run_id.to_string());
        });
        (
            RunEventObserverHandle(Some(observer)),
            Some(Arc::new(Self {
                cancellation,
                handle,
            })),
        )
    }
}

impl Drop for RunEventPublisher {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.handle.abort();
    }
}

/// Drains run-event notifications, coalescing bursts per run (~120ms) before
/// publishing journal deltas, run.upserts, and terminal trigger events
/// through publish_live_run_after.
async fn run_event_publisher_loop(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    mut receiver: mpsc::UnboundedReceiver<String>,
    cancellation: CancellationToken,
) {
    let mut last_published_sequence: HashMap<String, u64> = HashMap::new();
    let mut usage_accumulators: HashMap<String, RunUsageAccumulator> = HashMap::new();
    loop {
        let first = tokio::select! {
            _ = cancellation.cancelled() => return,
            received = receiver.recv() => received,
        };
        let Some(first) = first else {
            return;
        };
        let mut pending: BTreeSet<String> = BTreeSet::new();
        pending.insert(first);
        // Coalesce the burst: chatty executors notify per journal append.
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = time::sleep(Duration::from_millis(120)) => {}
        }
        while let Ok(run_id) = receiver.try_recv() {
            pending.insert(run_id);
        }
        for run_id in pending {
            let before_sequence = last_published_sequence.get(&run_id).copied();
            let publish_settings = settings.clone();
            let publish_hub = live_hub.clone();
            let publish_run_id = run_id.clone();
            let mut usage = usage_accumulators.remove(&run_id);
            let published = tokio::task::spawn_blocking(move || {
                let sequence = publish_live_run_after_incremental(
                    &publish_settings,
                    &publish_hub,
                    &publish_run_id,
                    before_sequence,
                    &mut usage,
                );
                (sequence, usage)
            })
            .await
            .ok();
            let Some((published_through, usage)) = published else {
                continue;
            };
            if let Some(usage) = usage {
                if let Some(sequence) = published_through {
                    last_published_sequence.insert(run_id.clone(), sequence);
                }
                usage_accumulators.insert(run_id, usage);
            } else {
                last_published_sequence.remove(&run_id);
            }
        }
    }
}

struct TriggerSourceLoop {
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

impl TriggerSourceLoop {
    fn spawn(settings: Arc<SparkSettings>, live_hub: Arc<WorkspaceLiveHub>) -> Option<Arc<Self>> {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return None;
        };
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_trigger_source_loop(
            settings,
            live_hub,
            task_cancellation,
        ));
        Some(Arc::new(Self {
            cancellation,
            handle,
        }))
    }
}

impl Drop for TriggerSourceLoop {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.handle.abort();
    }
}

impl HttpAppState {
    fn with_trigger_source_loop(mut self) -> Self {
        self.trigger_source_loop =
            TriggerSourceLoop::spawn(self.settings.clone(), self.live_hub.clone());
        self
    }
}

pub struct WorkspaceApiError(pub WorkspaceError);

impl IntoResponse for WorkspaceApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(json!({ "detail": self.0.detail() }))).into_response()
    }
}

impl From<WorkspaceError> for WorkspaceApiError {
    fn from(value: WorkspaceError) -> Self {
        Self(value)
    }
}

async fn workspace_api_fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Not Found" })),
    )
}

async fn serve_index(State(settings): State<Arc<SparkSettings>>) -> Response {
    let Some(resource) = spark_assets::frontend::load_index(&settings) else {
        return json_detail(StatusCode::NOT_FOUND, "UI index not found");
    };
    serve_resource(resource, "text/html; charset=utf-8")
}

async fn serve_favicon(State(settings): State<Arc<SparkSettings>>) -> Response {
    let Some(resource) = spark_assets::frontend::load_favicon(&settings) else {
        return json_detail(StatusCode::NOT_FOUND, "Asset not found");
    };
    serve_resource(resource, "image/png")
}

async fn serve_asset(
    State(settings): State<Arc<SparkSettings>>,
    Path(asset_path): Path<String>,
) -> Response {
    match spark_assets::frontend::load_asset(&settings, &format!("assets/{asset_path}")) {
        Ok(Some(resource)) => {
            let content_type = content_type_for_asset(resource.logical_path());
            serve_resource(resource, content_type)
        }
        Ok(None) | Err(_) => json_detail(StatusCode::NOT_FOUND, "Asset not found"),
    }
}

fn serve_resource(resource: spark_assets::ResourceFile, content_type: &'static str) -> Response {
    let mut response = (StatusCode::OK, Bytes::copy_from_slice(resource.bytes())).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn product_fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Not Found" })),
    )
}

async fn redirect_workspace_mount(headers: HeaderMap, uri: Uri) -> Response {
    redirect_to_mount_slash(headers, &uri, "/workspace/")
}

async fn redirect_attractor_mount(headers: HeaderMap, uri: Uri) -> Response {
    redirect_to_mount_slash(headers, &uri, "/attractor/")
}

fn redirect_to_mount_slash(headers: HeaderMap, uri: &Uri, target_path: &'static str) -> Response {
    let mut target = target_path.to_string();
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }
    let location = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| format!("http://{host}{target}"))
        .unwrap_or(target);

    let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
    if let Ok(location) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

fn json_detail(status: StatusCode, detail: &'static str) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

fn content_type_for_asset(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("json") | Some("map") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

async fn attractor_dispatch(
    State(state): State<HttpAppState>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let settings = state.settings.clone();
    let live_hub = state.live_hub.clone();
    let path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or_else(|| uri.path());
    let path_run_id = is_mutating_method(&method)
        .then(|| pipeline_id_from_attractor_path(path))
        .flatten();
    let path_run_sequence = path_run_id
        .as_deref()
        .and_then(|run_id| latest_run_sequence(&settings, run_id).ok().flatten());
    let body = String::from_utf8_lossy(&body);
    let response = attractor_api::handle_attractor_request_with_options(
        method.as_str(),
        path,
        &body,
        (*settings).clone(),
        state.runtime_handler_runner_factory.clone(),
        state.run_event_observer.0.clone(),
    );
    if is_mutating_method(&method) && response.status_code < 400 {
        publish_attractor_live_updates(
            &settings,
            &live_hub,
            path_run_id.as_deref(),
            path_run_sequence,
            &response.body,
        );
    }
    let content_type = response.content_type.clone();
    let status =
        StatusCode::from_u16(response.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = if content_type.starts_with("application/json") {
        (status, Json(response.body)).into_response()
    } else {
        let body = response
            .body
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| response.body.to_string());
        (status, body).into_response()
    };
    if let Ok(content_type) = HeaderValue::from_str(&content_type) {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        response
            .headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    }
    response
}

/// Publishes journal, segment, upsert, and milestone envelopes for the run.
/// Returns the journal's newest sequence from the same combined-journal
/// build (the envelopes already include the cursor window's segment
/// upserts), so callers advance their cursor without re-deriving the
/// journal — that rebuild used to double the cost of every publish cycle.
pub(crate) fn publish_live_run_after(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
) -> Option<u64> {
    let mut usage = None;
    publish_live_run_after_incremental(settings, live_hub, run_id, before_sequence, &mut usage)
}

fn publish_live_run_after_incremental(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
) -> Option<u64> {
    publish_live_run_after_incremental_with(
        settings,
        live_hub,
        run_id,
        before_sequence,
        usage,
        || full_run_usage_accumulator(settings, run_id).ok(),
    )
}

fn publish_live_run_after_incremental_with(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
    mut initialize_full: impl FnMut() -> Option<RunUsageAccumulator>,
) -> Option<u64> {
    let publication = run_live_publication(settings, run_id, before_sequence)
        .ok()
        .flatten();
    let (new_run_envelopes, latest_sequence) = match publication {
        Some(publication) => {
            let mut initialized_from_full_projection = false;
            if usage.is_none() {
                *usage = if publication.starts_at_sequence_zero {
                    Some(RunUsageAccumulator::new(&publication.fallback_model))
                } else {
                    initialized_from_full_projection = true;
                    initialize_full()
                };
            }
            if !initialized_from_full_projection {
                if let Some(usage) = usage.as_mut() {
                    usage.apply(&publication.newly_read_entries);
                }
            }
            (publication.envelopes, publication.latest_sequence)
        }
        None => (Vec::new(), None),
    };
    for envelope in &new_run_envelopes {
        live_hub.publish(envelope.clone());
    }
    let breakdown = usage.as_ref().and_then(|usage| usage.breakdown());
    if let Ok(Some(envelope)) = run_upsert_envelope_with_usage(settings, run_id, breakdown.as_ref())
    {
        live_hub.publish(envelope);
    }
    // Best-effort: workflow log projection must never fail the publish path.
    if let Ok(entries) =
        spark_workspace::project_run_milestones(settings, run_id, &new_run_envelopes)
    {
        for entry in &entries {
            live_hub.publish(spark_workspace::workflow_log_envelope(entry));
        }
    }
    publish_terminal_run_trigger_events(settings, live_hub, run_id);
    if spark_workspace::live::evict_run_live_cache_if_terminal(settings, run_id) {
        *usage = None;
    }
    latest_sequence
}

#[cfg(test)]
mod incremental_usage_tests {
    use super::*;
    use attractor_core::{RawRuntimeEvent, RunRecord};
    use attractor_runtime::{CreateRunRequest, RunStore};
    use serde_json::json;
    use std::fs;

    fn test_settings(root: &std::path::Path) -> SparkSettings {
        let data = root.join("data");
        SparkSettings {
            project_root: root.to_path_buf(),
            data_dir: data.clone(),
            config_dir: data.join("config"),
            runtime_dir: data.join("runtime"),
            logs_dir: data.join("logs"),
            workspace_dir: data.join("workspace"),
            projects_dir: data.join("workspace/projects"),
            attractor_dir: data.join("attractor"),
            runs_dir: data.join("attractor/runs"),
            flows_dir: data.join("flows"),
            ui_dir: None,
            project_roots: vec![root.to_path_buf()],
        }
    }

    fn event(run_id: &str, node: &str, event_type: &str, total: u64) -> RawRuntimeEvent {
        let mut event = RawRuntimeEvent::new("CodergenAdapter", run_id);
        event.payload = serde_json::from_value(json!({
            "node_id": node,
            "adapter_event_type": event_type,
            "payload": {"model": "gpt-5", "token_usage": {"input_tokens": total - 1, "output_tokens": 1}}
        })).expect("payload");
        event
    }

    fn upsert(receiver: &mut broadcast::Receiver<LiveEnvelope>) -> Value {
        loop {
            let envelope = receiver.try_recv().expect("published envelope");
            if envelope.event_type == "run.upsert" {
                return envelope.payload["run"].clone();
            }
        }
    }

    #[test]
    fn real_publisher_overlays_only_new_usage_and_evicts_terminal_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = test_settings(temp.path());
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project");
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("live-usage", project.to_string_lossy().into_owned());
        record.model = "gpt-5".into();
        record.status = "running".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .expect("run");
        let hub = WorkspaceLiveHub::new();
        let mut receiver = hub.subscribe();
        let mut usage = None;

        store
            .append_event(
                &paths,
                event("live-usage", "work", "codex_app_server_session_event", 10),
            )
            .expect("event");
        let cursor =
            publish_live_run_after_incremental(&settings, &hub, "live-usage", None, &mut usage);
        let first = upsert(&mut receiver);
        assert_eq!(first["token_usage"], 10);
        assert_eq!(
            first["token_usage_breakdown"]["by_model"]["gpt-5"]["total_tokens"],
            10
        );
        assert_eq!(first["estimated_model_cost"]["status"], "estimated");
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().token_usage,
            None,
            "live overlay must not write run.json"
        );

        // Re-publishing the same cursor applies no entry twice.
        publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert_eq!(upsert(&mut receiver)["token_usage"], 10);
        store
            .append_event(
                &paths,
                event("live-usage", "work", "codex_app_server_session_event", 20),
            )
            .expect("event");
        let cursor =
            publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert_eq!(upsert(&mut receiver)["token_usage"], 20);

        // A fresh publisher (reconnect/restart) reconstructs the same full projection.
        let mut restarted = None;
        publish_live_run_after_incremental(&settings, &hub, "live-usage", None, &mut restarted);
        assert_eq!(upsert(&mut receiver)["token_usage"], 20);

        store
            .update_run_record("live-usage", |record| {
                record.status = "completed".into();
                record.token_usage = Some(20);
            })
            .expect("terminal");
        publish_live_run_after_incremental(&settings, &hub, "live-usage", cursor, &mut usage);
        assert!(usage.is_none(), "terminal publication evicts accumulator");
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().token_usage,
            Some(20)
        );
    }

    #[test]
    fn incomplete_bounded_history_initializes_full_projection_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = test_settings(temp.path());
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project");
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("bounded-usage", project.to_string_lossy().into_owned());
        record.model = "gpt-5".into();
        record.status = "running".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .expect("run");
        store
            .append_event(
                &paths,
                event("bounded-usage", "work", "codex_app_server_session_event", 7),
            )
            .expect("usage");
        for _ in 0..4097 {
            let mut noise = RawRuntimeEvent::new("Log", "bounded-usage");
            noise.payload.insert("message".into(), json!("noise"));
            store.append_event(&paths, noise).expect("noise");
        }
        let hub = WorkspaceLiveHub::new();
        let mut usage = None;
        let mut initializations = 0;
        let mut initialize = || {
            initializations += 1;
            full_run_usage_accumulator(&settings, "bounded-usage").ok()
        };
        let cursor = publish_live_run_after_incremental_with(
            &settings,
            &hub,
            "bounded-usage",
            Some(1),
            &mut usage,
            &mut initialize,
        );
        publish_live_run_after_incremental_with(
            &settings,
            &hub,
            "bounded-usage",
            cursor,
            &mut usage,
            &mut initialize,
        );
        drop(initialize);
        assert_eq!(initializations, 1);
        assert_eq!(
            usage
                .and_then(|usage| usage.breakdown())
                .unwrap()
                .totals
                .total_tokens,
            7
        );
    }
}

async fn run_trigger_source_loop(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    cancellation: CancellationToken,
) {
    let mut interval = time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => {
                let service = WorkspaceTriggerService::new((*settings).clone());
                if let Ok(outcomes) = service.process_due_trigger_sources().await {
                    publish_trigger_activation_outcomes(&settings, &live_hub, outcomes);
                }
            }
        }
    }
}

fn publish_terminal_run_trigger_events(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
) {
    if let Ok(outcomes) =
        WorkspaceTriggerService::new(settings.clone()).emit_terminal_flow_event_for_run(run_id)
    {
        publish_trigger_activation_outcomes(settings, live_hub, outcomes);
    }
}

pub(crate) fn publish_trigger_activation_outcomes(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    outcomes: Vec<spark_workspace::TriggerActivationOutcome>,
) {
    let mut run_ids = BTreeSet::new();
    for outcome in outcomes {
        if let Ok(value) = serde_json::to_value(&outcome.trigger) {
            live_hub.publish(trigger_upsert_envelope(&value));
        }
        if let Some(run_id) = outcome.run_id {
            run_ids.insert(run_id);
        }
    }
    for run_id in run_ids {
        publish_live_run_after(settings, live_hub, &run_id, None);
    }
}

fn publish_attractor_live_updates(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    path_run_id: Option<&str>,
    path_run_sequence: Option<u64>,
    body: &Value,
) {
    let mut run_ids = BTreeSet::new();
    if let Some(run_id) = path_run_id {
        run_ids.insert(run_id.to_string());
    }
    collect_run_ids_from_value(body, &mut run_ids);
    for run_id in run_ids {
        let before_sequence = if Some(run_id.as_str()) == path_run_id {
            path_run_sequence
        } else {
            None
        };
        publish_live_run_after(settings, live_hub, &run_id, before_sequence);
    }
}

fn collect_run_ids_from_value(value: &Value, run_ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "run_id" | "pipeline_id" | "source_run_id" | "result_run_id" | "target_run_id"
                ) {
                    if let Some(run_id) = value.as_str().filter(|value| !value.trim().is_empty()) {
                        run_ids.insert(run_id.to_string());
                    }
                }
                collect_run_ids_from_value(value, run_ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_run_ids_from_value(value, run_ids);
            }
        }
        _ => {}
    }
}

fn pipeline_id_from_attractor_path(path: &str) -> Option<String> {
    let path = path.split('?').next().unwrap_or(path);
    let rest = path.strip_prefix("/attractor/pipelines/")?;
    let pipeline_id = rest.split('/').next().unwrap_or_default().trim();
    (!pipeline_id.is_empty()).then(|| pipeline_id.to_string())
}

fn is_mutating_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}
