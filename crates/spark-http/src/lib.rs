#![forbid(unsafe_code)]

//! Axum HTTP composition for Spark Workspace compatibility routes.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use serde_json::json;
use serde_json::Value;
use spark_common::settings::SparkSettings;
use spark_workspace::live::{
    latest_run_sequence, run_envelopes_after, run_upsert_envelope, trigger_upsert_envelope,
    LiveEnvelope,
};
use spark_workspace::{WorkspaceError, WorkspaceTriggerService};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;

mod workspace;

pub fn build_app(settings: SparkSettings) -> Router {
    build_app_with_runtime_handler_runner_factory(
        settings,
        attractor_api::default_runtime_handler_runner_factory(),
    )
}

pub fn build_app_with_rust_llm_client(
    settings: SparkSettings,
    client: unified_llm_adapter::Client,
) -> Router {
    build_app_with_runtime_handler_runner_factory(
        settings,
        attractor_api::rust_llm_runtime_handler_runner_factory(client),
    )
}

pub fn build_app_with_runtime_handler_runner_factory(
    settings: SparkSettings,
    runtime_handler_runner_factory: attractor_api::RuntimeHandlerRunnerFactory,
) -> Router {
    let state = HttpAppState {
        settings: Arc::new(settings),
        live_hub: Arc::new(WorkspaceLiveHub::new()),
        runtime_handler_runner_factory,
        trigger_source_loop: None,
    };
    let state = state.with_trigger_source_loop();
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
    #[allow(dead_code)]
    trigger_source_loop: Option<Arc<TriggerSourceLoop>>,
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
    let response = attractor_api::handle_attractor_request_with_runtime_handler_runner_factory(
        method.as_str(),
        path,
        &body,
        (*settings).clone(),
        state.runtime_handler_runner_factory.clone(),
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

pub(crate) fn publish_live_run_after(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    run_id: &str,
    before_sequence: Option<u64>,
) {
    let after_sequence = before_sequence.unwrap_or(0);
    if let Ok(envelopes) = run_envelopes_after(settings, run_id, after_sequence) {
        for envelope in envelopes {
            live_hub.publish(envelope);
        }
    }
    if let Ok(Some(envelope)) = run_upsert_envelope(settings, run_id) {
        live_hub.publish(envelope);
    }
    publish_terminal_run_trigger_events(settings, live_hub, run_id);
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
