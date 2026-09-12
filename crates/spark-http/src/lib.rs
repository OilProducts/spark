#![forbid(unsafe_code)]

//! Axum HTTP composition for Spark Workspace compatibility routes.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

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
    execution_transcript_envelopes, full_run_usage_accumulator, latest_run_sequence,
    run_live_publication, run_upsert_envelope_with_usage, trigger_upsert_envelope,
    ExecutionTranscriptCursor, LiveEnvelope, RunUsageAccumulator,
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

/// Native settings writes share the HTTP app's existing live transport.
pub fn build_app_with_rust_llm_client_and_settings_notifications(
    settings: SparkSettings,
    client: unified_llm_adapter::Client,
) -> (Router, impl Fn(&str) + Send + Sync + 'static) {
    let live_hub = Arc::new(WorkspaceLiveHub::new());
    let publisher = live_hub.clone();
    let app = build_app_with_live_hub(
        settings,
        attractor_api::rust_llm_runtime_handler_runner_factory(client.clone()),
        Arc::new(RustLlmAgentTurnBackend::new(client)),
        live_hub,
    );
    (app, move |revision| {
        publisher.publish_settings_change(
            "workspace",
            "desktop",
            None,
            serde_json::json!(revision),
        );
    })
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
    build_app_with_live_hub(
        settings,
        runtime_handler_runner_factory,
        agent_turn_backend,
        Arc::new(WorkspaceLiveHub::new()),
    )
}

fn build_app_with_live_hub(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
    live_hub: Arc<WorkspaceLiveHub>,
) -> Router {
    let settings = Arc::new(settings);
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
        .route(
            "/attractor/api/llm-profiles",
            get(attractor_dispatch).patch(workspace::patch_llm_profiles),
        )
        .route(
            "/attractor/api/execution-placement-settings",
            get(attractor_dispatch).patch(workspace::patch_execution_profiles),
        )
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
    execution_cursors: Mutex<HashMap<String, ExecutionTranscriptCursor>>,
}

impl WorkspaceLiveHub {
    fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(256);
        Self {
            sender,
            execution_cursors: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<LiveEnvelope> {
        self.sender.subscribe()
    }

    pub(crate) fn publish_settings_change(
        &self,
        scope: &str,
        section: &str,
        project_path: Option<String>,
        revision: serde_json::Value,
    ) {
        self.publish(LiveEnvelope {
            event_type: "settings.changed".into(),
            project_path,
            resource: spark_workspace::LiveResource {
                kind: "settings".into(),
                id: Some(scope.into()),
            },
            cursor: None,
            reason: None,
            payload: serde_json::json!({"scope": scope, "section": section, "revision": revision}),
        });
    }

    pub(crate) fn publish(&self, envelope: LiveEnvelope) {
        let _ = self.sender.send(envelope);
    }

    fn publish_execution_transcripts(&self, settings: &SparkSettings, run_id: &str) {
        let mut cursors = self
            .execution_cursors
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Ok(envelopes) = execution_transcript_envelopes(settings, run_id, &mut cursors) {
            for envelope in envelopes {
                self.publish(envelope);
            }
        }
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
        let observer: attractor_api::RunEventObserver = Arc::new(move |run_id: &str| {
            let _ = sender.send(run_id.to_string());
        });
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_event_publisher_loop(
            settings,
            live_hub,
            receiver,
            task_cancellation,
            observer.clone(),
        ));
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
    run_event_observer: attractor_api::RunEventObserver,
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
            let publish_observer = run_event_observer.clone();
            let mut usage = usage_accumulators.remove(&run_id);
            let published = tokio::task::spawn_blocking(move || {
                let sequence = publish_live_run_after_incremental_with_observer(
                    &publish_settings,
                    &publish_hub,
                    &publish_run_id,
                    before_sequence,
                    &mut usage,
                    Some(publish_observer),
                );
                (sequence, usage)
            })
            .await
            .ok();
            let Some((published_through, usage)) = published else {
                continue;
            };
            // ponytail: retain one cursor per observed run until publisher shutdown.
            if let Some(sequence) = published_through {
                last_published_sequence.insert(run_id.clone(), sequence);
            }
            if let Some(usage) = usage {
                usage_accumulators.insert(run_id, usage);
            }
        }
    }
}

struct TriggerSourceLoop {
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

impl TriggerSourceLoop {
    fn spawn(
        settings: Arc<SparkSettings>,
        live_hub: Arc<WorkspaceLiveHub>,
        run_event_observer: RunEventObserverHandle,
    ) -> Option<Arc<Self>> {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return None;
        };
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let handle = handle.spawn(run_trigger_source_loop(
            settings,
            live_hub,
            task_cancellation,
            run_event_observer,
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
        self.trigger_source_loop = TriggerSourceLoop::spawn(
            self.settings.clone(),
            self.live_hub.clone(),
            self.run_event_observer.clone(),
        );
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

/// Publishes orchestration envelopes and independently owned execution
/// transcript upserts. Returns the run journal's newest local sequence.
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
    publish_live_run_after_incremental_with_observer(
        settings,
        live_hub,
        run_id,
        before_sequence,
        usage,
        None,
    )
}

fn publish_live_run_after_incremental_with_observer(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
    run_event_observer: Option<attractor_api::RunEventObserver>,
) -> Option<u64> {
    publish_live_run_after_incremental_with(
        settings,
        live_hub,
        run_id,
        before_sequence,
        usage,
        run_event_observer,
        || full_run_usage_accumulator(settings, run_id).ok(),
    )
}

fn publish_live_run_after_incremental_with(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
    usage: &mut Option<RunUsageAccumulator>,
    run_event_observer: Option<attractor_api::RunEventObserver>,
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
    live_hub.publish_execution_transcripts(settings, run_id);
    *usage = full_run_usage_accumulator(settings, run_id).ok();
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
    publish_terminal_run_trigger_events(settings, live_hub, run_id, run_event_observer);
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
    use serde_json::{json, Map};
    use std::fs;
    use std::sync::Mutex;

    fn test_settings(root: &std::path::Path) -> SparkSettings {
        let data = root.join("data");
        SparkSettings {
            connections: Default::default(),
            providers: Default::default(),
            agents: Default::default(),
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

    fn append_execution_usage(paths: &attractor_runtime::RunRootPaths, event: RawRuntimeEvent) {
        let root = paths.logs_dir().join("work/executions/1-0");
        fs::create_dir_all(&root).expect("execution root");
        fs::write(root.join("status.json"), r#"{"outcome":"running"}"#).expect("status");
        let event_type = event.payload["adapter_event_type"].clone();
        let mut payload = event.payload["payload"].clone();
        payload["node_id"] = event.payload["node_id"].clone();
        spark_storage::ActivityRepository::new(root)
            .append_event(json!({"event_type": event_type, "payload": payload}), "now")
            .expect("execution usage");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_run_notifications_do_not_replay_history() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Arc::new(test_settings(temp.path()));
        let store = RunStore::for_settings(&settings);
        let mut record = RunRecord::new("failed-live", temp.path().to_string_lossy().into_owned());
        record.status = "failed".into();
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap();
        for _ in 0..300 {
            store
                .append_event(&paths, RawRuntimeEvent::new("Log", "failed-live"))
                .unwrap();
        }
        let hub = Arc::new(WorkspaceLiveHub::new());
        let mut receiver = hub.subscribe();
        let (sender, notifications) = mpsc::unbounded_channel();
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_event_publisher_loop(
            settings.clone(),
            hub,
            notifications,
            cancellation.clone(),
            Arc::new(|_| {}),
        ));
        // Count dropped broadcast entries too: the initial history exceeds the live buffer.
        async fn batch(receiver: &mut broadcast::Receiver<LiveEnvelope>) -> u64 {
            let mut count = 0;
            loop {
                match time::timeout(Duration::from_secs(5), receiver.recv())
                    .await
                    .unwrap()
                {
                    Ok(envelope) if envelope.event_type == "run.upsert" => return count,
                    Ok(envelope) if envelope.event_type == "run.journal_entry" => count += 1,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => count += n,
                    Err(error) => panic!("{error}"),
                }
            }
        }
        sender.send("failed-live".into()).unwrap();
        assert_eq!(
            batch(&mut receiver).await,
            latest_run_sequence(&settings, "failed-live")
                .unwrap()
                .unwrap()
        );
        for _ in 0..3 {
            sender.send("failed-live".into()).unwrap();
            assert_eq!(
                batch(&mut receiver).await,
                0,
                "terminal usage eviction must retain publication progress"
            );
        }
        let history = fs::read(paths.events_jsonl()).unwrap();
        // Publication errors and empty journals must not erase the previous cursor.
        for contents in [b"invalid json\n".as_slice(), b"".as_slice()] {
            fs::write(paths.events_jsonl(), contents).unwrap();
            assert!(run_live_publication(&settings, "failed-live", None)
                .ok()
                .flatten()
                .and_then(|publication| publication.latest_sequence)
                .is_none());
            sender.send("failed-live".into()).unwrap();
            assert_eq!(batch(&mut receiver).await, 0);
            fs::write(paths.events_jsonl(), &history).unwrap();
            sender.send("failed-live".into()).unwrap();
            assert_eq!(
                batch(&mut receiver).await,
                0,
                "publication failure must preserve cursor"
            );
        }
        store
            .append_event(&paths, RawRuntimeEvent::new("Log", "failed-live"))
            .unwrap();
        sender.send("failed-live".into()).unwrap();
        assert_eq!(batch(&mut receiver).await, 1);
        sender.send("failed-live".into()).unwrap();
        assert_eq!(batch(&mut receiver).await, 0);
        cancellation.cancel();
        task.await.unwrap();
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

        append_execution_usage(
            &paths,
            event("live-usage", "work", "codex_app_server_session_event", 10),
        );
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
        append_execution_usage(
            &paths,
            event("live-usage", "work", "codex_app_server_session_event", 20),
        );
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
        append_execution_usage(
            &paths,
            event("bounded-usage", "work", "codex_app_server_session_event", 7),
        );
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
            None,
            &mut initialize,
        );
        publish_live_run_after_incremental_with(
            &settings,
            &hub,
            "bounded-usage",
            cursor,
            &mut usage,
            None,
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

    #[test]
    fn http_terminal_publisher_launches_observed_successor_once_for_duplicate_events() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = test_settings(temp.path());
        let project = temp.path().join("project");
        let mathlab = project.join(".mathlab");
        fs::create_dir_all(&mathlab).expect("mathlab");
        let flow_name = "math-research/prove-refute.yaml";
        let flow_path = settings.flows_dir.join(flow_name);
        fs::create_dir_all(flow_path.parent().expect("flow parent")).expect("flow parent");
        fs::write(
            flow_path,
            "schema_version: '1'\nid: math-chain\ninputs:\n  - key: context.request.problem\n    type: string\n    required: true\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
        )
        .expect("flow");
        fs::write(
            mathlab.join("next-session.json"),
            serde_json::to_vec(&json!({
                "flow_name": flow_name,
                "inputs": {"context.request.problem": "P"}
            }))
            .expect("draft json"),
        )
        .expect("draft");
        spark_storage::TriggerRepositories::from_settings(&settings)
            .definitions
            .put(&spark_storage::TriggerDefinition {
                revision: String::new(),
                id: "math-http-chain".into(),
                name: "Math HTTP chain".into(),
                enabled: true,
                protected: false,
                source_type: "flow_event".into(),
                action: spark_storage::TriggerAction {
                    mode: "workspace_draft".into(),
                    flow_name: flow_name.into(),
                    project_path: Some(project.to_string_lossy().into_owned()),
                    static_context: Map::new(),
                    flow_allowlist: vec!["math-research/*".into()],
                    execution_profile_id: None,
                },
                source: Map::from_iter([
                    ("flow_name".into(), json!(flow_name)),
                    ("statuses".into(), json!(["completed"])),
                ]),
                created_at: "2026-07-23T00:00:00Z".into(),
                updated_at: "2026-07-23T00:00:00Z".into(),
            })
            .expect("trigger");
        let store = RunStore::for_settings(&settings);
        let mut terminal = RunRecord::new("terminal-source", project.to_string_lossy());
        terminal.flow_name = flow_name.into();
        terminal.status = "completed".into();
        store
            .create_run(CreateRunRequest {
                record: terminal,
                ..Default::default()
            })
            .expect("terminal run");
        let observed = Arc::new(Mutex::new(Vec::new()));
        let captured = observed.clone();
        let observer: attractor_api::RunEventObserver = Arc::new(move |run_id| {
            captured.lock().expect("observed").push(run_id.to_string());
        });
        let hub = WorkspaceLiveHub::new();
        let mut usage = None;

        publish_live_run_after_incremental_with_observer(
            &settings,
            &hub,
            "terminal-source",
            None,
            &mut usage,
            Some(observer.clone()),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && observed.lock().expect("observed").is_empty()
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let successor_ids = observed
            .lock()
            .expect("observed")
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(successor_ids.len(), 1, "observed {successor_ids:?}");
        assert!(store
            .find_run_root(successor_ids.first().expect("successor"))
            .expect("find successor")
            .is_some());

        publish_live_run_after_incremental_with_observer(
            &settings,
            &hub,
            "terminal-source",
            None,
            &mut usage,
            Some(observer),
        );
        assert_eq!(
            observed
                .lock()
                .expect("observed")
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            successor_ids,
            "duplicate terminal publication must not launch another successor"
        );
    }
}

async fn run_trigger_source_loop(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    cancellation: CancellationToken,
    run_event_observer: RunEventObserverHandle,
) {
    let mut interval = time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => {
                let mut service = WorkspaceTriggerService::new((*settings).clone());
                if let Some(observer) = &run_event_observer.0 {
                    service = service.with_run_event_observer(observer.clone());
                }
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
    run_event_observer: Option<attractor_api::RunEventObserver>,
) {
    let mut service = WorkspaceTriggerService::new(settings.clone());
    if let Some(observer) = run_event_observer {
        service = service.with_run_event_observer(observer);
    }
    if let Ok(outcomes) = service.emit_terminal_flow_event_for_run(run_id) {
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
