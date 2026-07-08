#![forbid(unsafe_code)]

//! Minimal Attractor API surface for Rust rewrite compatibility milestones.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use attractor_core::{ContextMap, DotGraph, LaunchContext, RawRuntimeEvent, RunRecord};
use attractor_dsl::{
    apply_graph_transforms, diagnostics_payload, ensure_flows_dir, flow_name_from_path,
    format_readable_dot, inject_pipeline_goal, load_flow_content, parse_dot,
    preview_response_payload_with_options, resolve_flow_path, semantic_signature_for_source,
    validate_graph, Diagnostic, DiagnosticSeverity, DotParseError, FlowSourceError, PreviewOptions,
};
pub use attractor_runtime::RunEventObserver;
use attractor_runtime::{
    human_intervention_requested_event, ContinueRunRequest, ExecuteRunRequest, ExecutionStart,
    PipelineExecutor, RunBundle, RunStore, RuntimeControlError, RuntimeControls,
    RuntimeHandlerRunner,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

pub type RuntimeHandlerRunnerFactory = Arc<dyn Fn() -> RuntimeHandlerRunner + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewRequest {
    pub flow_content: String,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub expand_children: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewServiceConfig {
    pub flows_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewRouteResponse {
    pub status_code: u16,
    pub content_type: String,
    pub body: Value,
}

impl PreviewRouteResponse {
    pub fn json(status_code: u16, body: Value) -> Self {
        Self {
            status_code,
            content_type: "application/json".to_string(),
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRouteResponse {
    pub status_code: u16,
    pub content_type: String,
    pub body: Value,
}

impl RuntimeRouteResponse {
    pub fn json(status_code: u16, body: Value) -> Self {
        Self {
            status_code,
            content_type: "application/json".to_string(),
            body,
        }
    }

    pub fn text(status_code: u16, body: impl Into<String>) -> Self {
        Self {
            status_code,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: Value::String(body.into()),
        }
    }

    pub fn event_stream(status_code: u16, body: impl Into<String>) -> Self {
        Self {
            status_code,
            content_type: "text/event-stream".to_string(),
            body: Value::String(body.into()),
        }
    }

    pub fn file(
        status_code: u16,
        content_type: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            status_code,
            content_type: content_type.into(),
            body: Value::String(body.into()),
        }
    }

    pub fn with_not_found_detail(mut self, detail: &str) -> Self {
        if self.status_code == 404 && self.body.get("detail").is_some() {
            self.body = json!({"detail": detail});
        }
        self
    }

    pub fn with_content_type(mut self, content_type: &str) -> Self {
        if self.status_code == 200 {
            self.content_type = content_type.to_string();
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedFlowSource {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveFlowRequest {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub expect_semantic_equivalence: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContinuePipelineRequest {
    pub start_node: String,
    pub flow_source_mode: String,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineStartRequest {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub flow_content: Option<String>,
    #[serde(default = "default_working_directory")]
    pub working_directory: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
    #[serde(default)]
    pub project_default_execution_profile_id: Option<String>,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub launch_context: Option<ContextMap>,
    #[serde(default)]
    pub spec_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    /// When true, execute the pipeline inline and only respond at a terminal
    /// state (the pre-detached behavior). Defaults to detached execution.
    #[serde(default)]
    pub wait: Option<bool>,
}

impl Default for PipelineStartRequest {
    fn default() -> Self {
        Self {
            run_id: None,
            flow_content: None,
            working_directory: default_working_directory(),
            model: None,
            llm_provider: None,
            llm_profile: None,
            reasoning_effort: None,
            execution_profile_id: None,
            project_default_execution_profile_id: None,
            flow_name: None,
            goal: None,
            launch_context: None,
            spec_id: None,
            plan_id: None,
            wait: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineSteerRequest {
    pub message: String,
    #[serde(default)]
    pub target_run_id: Option<String>,
    #[serde(default)]
    pub target_node_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineMetadataUpdateRequest {
    #[serde(default)]
    pub spec_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanAnswerRequest {
    pub selected_value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedContinueFlow {
    pub graph: DotGraph,
    #[serde(default)]
    pub graph_source: Option<String>,
    #[serde(default)]
    pub graph_dot: Option<String>,
    #[serde(default)]
    pub flow_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeControlService {
    controls: RuntimeControls,
}

impl RuntimeControlService {
    pub fn new(store: RunStore) -> Self {
        Self {
            controls: RuntimeControls::new(store),
        }
    }

    pub fn from_controls(controls: RuntimeControls) -> Self {
        Self { controls }
    }

    pub fn get_checkpoint(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match self.controls.get_checkpoint(pipeline_id) {
            Ok(checkpoint) => RuntimeRouteResponse::json(
                200,
                json!({"pipeline_id": pipeline_id, "checkpoint": checkpoint}),
            ),
            Err(error) => runtime_error_response(error, "Checkpoint unavailable"),
        }
    }

    pub fn get_context(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match self.controls.get_context(pipeline_id) {
            Ok(context) => RuntimeRouteResponse::json(
                200,
                json!({"pipeline_id": pipeline_id, "context": context}),
            ),
            Err(error) => runtime_error_response(error, "Checkpoint unavailable"),
        }
    }

    pub fn continue_pipeline(
        &self,
        pipeline_id: &str,
        request: ContinuePipelineRequest,
        resolved_flow: ResolvedContinueFlow,
    ) -> RuntimeRouteResponse {
        let request = ContinueRunRequest {
            source_run_id: pipeline_id.to_string(),
            start_node: request.start_node,
            flow_source_mode: request.flow_source_mode,
            flow_name: request.flow_name.or(resolved_flow.flow_name),
            new_run_id: None,
            graph: resolved_flow.graph,
            graph_source: resolved_flow.graph_source,
            graph_dot: resolved_flow.graph_dot,
            working_directory: request.working_directory,
            model: request.model,
            llm_provider: request.llm_provider,
            llm_profile: request.llm_profile,
            reasoning_effort: request.reasoning_effort,
        };
        match self.controls.continue_from_snapshot(request) {
            Ok(started) => RuntimeRouteResponse::json(200, json!(started)),
            Err(error) => runtime_error_response(error, "Continue failed"),
        }
    }

    pub fn retry_pipeline(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match self.controls.prepare_retry(pipeline_id) {
            Ok(prepared) => RuntimeRouteResponse::json(200, json!(prepared)),
            Err(error) => runtime_error_response(error, "Retry failed"),
        }
    }

    pub fn cancel_pipeline(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match self.controls.request_cancel(pipeline_id) {
            Ok(status) => RuntimeRouteResponse::json(
                200,
                json!({
                    "status": status.status,
                    "pipeline_id": status.pipeline_id,
                }),
            ),
            Err(error) => runtime_error_response(error, "Cancel failed"),
        }
    }

    pub fn pause_pipeline(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match self.controls.request_pause(pipeline_id) {
            Ok(status) => RuntimeRouteResponse::json(
                200,
                json!({
                    "status": status.status,
                    "pipeline_id": status.pipeline_id,
                }),
            ),
            Err(error) => runtime_error_response(error, "Pause failed"),
        }
    }

    pub fn get_run_detail(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let store = self.controls.store();
        match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => {
                let result_available =
                    bundle.paths.result_json().exists() || bundle.paths.result_markdown().exists();
                match store.list_child_run_bundles(pipeline_id) {
                    Ok(children) => RuntimeRouteResponse::json(
                        200,
                        json!({
                            "pipeline_id": pipeline_id,
                            "record": bundle.record,
                            "checkpoint": bundle.checkpoint,
                            "result_available": result_available,
                            "raw_events": bundle.raw_events,
                            "journal": bundle.journal,
                            "child_runs": child_run_summaries(&children),
                        }),
                    ),
                    Err(error) => {
                        RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
                    }
                }
            }
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_run_journal(
        &self,
        pipeline_id: &str,
        include_children: bool,
    ) -> RuntimeRouteResponse {
        let store = self.controls.store();
        match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => {
                let child_journals = if include_children {
                    match store.list_child_run_bundles(pipeline_id) {
                        Ok(children) => child_journal_groups(&children),
                        Err(error) => {
                            return RuntimeRouteResponse::json(
                                500,
                                json!({"detail": error.to_string()}),
                            );
                        }
                    }
                } else {
                    Vec::new()
                };
                RuntimeRouteResponse::json(
                    200,
                    json!({
                        "pipeline_id": pipeline_id,
                        "journal": bundle.journal,
                        "child_journals": child_journals,
                    }),
                )
            }
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_run_events(
        &self,
        pipeline_id: &str,
        include_children: bool,
    ) -> RuntimeRouteResponse {
        let store = self.controls.store();
        match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => {
                let child_events = if include_children {
                    match store.list_child_run_bundles(pipeline_id) {
                        Ok(children) => child_event_groups(&children),
                        Err(error) => {
                            return RuntimeRouteResponse::json(
                                500,
                                json!({"detail": error.to_string()}),
                            );
                        }
                    }
                } else {
                    Vec::new()
                };
                RuntimeRouteResponse::json(
                    200,
                    json!({
                        "pipeline_id": pipeline_id,
                        "events": bundle.raw_events,
                        "child_events": child_events,
                    }),
                )
            }
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionPlacementService {
    settings: SparkSettings,
}

impl ExecutionPlacementService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }

    pub fn get_execution_placement_settings(&self) -> RuntimeRouteResponse {
        RuntimeRouteResponse::json(
            200,
            attractor_execution::public_execution_placement_settings(&self.settings),
        )
    }
}

pub fn execution_placement_settings(settings: &SparkSettings) -> RuntimeRouteResponse {
    RuntimeRouteResponse::json(
        200,
        attractor_execution::public_execution_placement_settings(settings),
    )
}

#[derive(Clone)]
pub struct AttractorApiService {
    settings: SparkSettings,
    runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}

impl AttractorApiService {
    pub fn new(settings: SparkSettings) -> Self {
        Self::new_with_runtime_handler_runner_factory(
            settings,
            default_runtime_handler_runner_factory(),
        )
    }

    pub fn new_with_runtime_handler_runner_factory(
        settings: SparkSettings,
        runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
    ) -> Self {
        Self {
            settings,
            runtime_handler_runner_factory,
            run_event_observer: None,
        }
    }

    pub fn with_run_event_observer(
        mut self,
        observer: attractor_runtime::RunEventObserver,
    ) -> Self {
        self.run_event_observer = Some(observer);
        self
    }

    fn observed_store(&self) -> RunStore {
        let store = RunStore::for_settings(&self.settings);
        match &self.run_event_observer {
            Some(observer) => store.with_run_event_observer(observer.clone()),
            None => store,
        }
    }

    fn observed_node_executor(&self) -> attractor_runtime::RuntimeHandlerRunner {
        let runner = (self.runtime_handler_runner_factory.as_ref())();
        match &self.run_event_observer {
            Some(observer) => runner.with_run_event_observer(observer.clone()),
            None => runner,
        }
    }

    /// Resumes an already-prepared run (continue/retry) on a background
    /// thread from its persisted record and checkpoint.
    fn spawn_prepared_resume(
        &self,
        run_id: &str,
        graph: DotGraph,
    ) -> std::result::Result<(), String> {
        let store = self.observed_store();
        let bundle = store
            .read_run_bundle(run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Unknown pipeline: {run_id}"))?;
        let record = bundle
            .record
            .ok_or_else(|| format!("Run record unavailable: {run_id}"))?;
        let checkpoint = bundle
            .checkpoint
            .ok_or_else(|| format!("Checkpoint unavailable: {run_id}"))?;
        let execute_request = ExecuteRunRequest {
            store: store.clone(),
            record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths: bundle.paths.clone(),
                checkpoint,
            },
        };
        self.spawn_detached_execution(bundle.paths, execute_request)
    }

    /// If a continue/retry route prepared a run successfully, execute it
    /// detached; failures to launch are persisted on the record so pollers
    /// and the live stream observe them.
    fn execute_prepared_route_response(&self, response: &RuntimeRouteResponse, graph: DotGraph) {
        if response.status_code >= 400 {
            return;
        }
        if response.body.get("status").and_then(Value::as_str) != Some("started") {
            return;
        }
        let Some(run_id) = response.body.get("run_id").and_then(Value::as_str) else {
            return;
        };
        if let Err(error) = self.spawn_prepared_resume(run_id, graph) {
            let _ = self.observed_store().update_run_record(run_id, |record| {
                record.status = "failed".to_string();
                record.last_error = error.clone();
            });
        }
    }

    /// Runs a prepared pipeline on a dedicated background thread, honoring
    /// persisted cancel/pause requests. Executor failures are recorded on the
    /// run record — the launch response has already been sent by the time
    /// they can occur.
    fn spawn_detached_execution(
        &self,
        paths: attractor_runtime::paths::RunRootPaths,
        execute_request: ExecuteRunRequest,
    ) -> std::result::Result<(), String> {
        let node_executor = self.observed_node_executor();
        let store = execute_request.store.clone();
        let run_id = paths.run_id.clone();
        std::thread::Builder::new()
            .name(format!("attractor-run-{run_id}"))
            .spawn(move || {
                let mut executor = PipelineExecutor::with_control(
                    node_executor,
                    attractor_runtime::disk_execution_control(paths),
                );
                if let Err(error) = executor.execute(execute_request) {
                    let message = error.to_string();
                    let _ = store.update_run_record(&run_id, |record| {
                        record.status = "failed".to_string();
                        record.last_error = message.clone();
                    });
                }
            })
            .map(|_handle| ())
            .map_err(|error| format!("Unable to spawn run execution thread: {error}"))
    }

    pub fn get_status(&self) -> RuntimeRouteResponse {
        RuntimeRouteResponse::json(
            200,
            json!({
                "status": "idle",
                "outcome": null,
                "outcome_reason_code": null,
                "outcome_reason_message": null,
                "last_error": "",
                "last_working_directory": "",
                "last_model": "",
                "last_completed_nodes": [],
                "last_flow_name": "",
            }),
        )
    }

    pub fn list_runs(&self) -> RuntimeRouteResponse {
        self.list_runs_for_project(None)
    }

    pub fn list_runs_for_project(&self, project_path: Option<&str>) -> RuntimeRouteResponse {
        match list_run_records(&self.settings, project_path) {
            Ok(runs) => RuntimeRouteResponse::json(200, json!({"runs": runs})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error})),
        }
    }

    pub fn deprecated_runs_events(&self) -> RuntimeRouteResponse {
        RuntimeRouteResponse::text(
            410,
            "Deprecated. Use /workspace/api/live/events with include_runs_overview=true.",
        )
    }

    pub fn preview(&self, req: PreviewRequest) -> RuntimeRouteResponse {
        let response = preview_with_config(
            req,
            &PreviewServiceConfig {
                flows_dir: Some(self.settings.flows_dir.clone()),
            },
        );
        RuntimeRouteResponse {
            status_code: response.status_code,
            content_type: response.content_type,
            body: response.body,
        }
    }

    pub fn list_flows(&self) -> RuntimeRouteResponse {
        match list_logical_flow_names(&self.settings.flows_dir) {
            Ok(names) => RuntimeRouteResponse::json(200, json!(names)),
            Err(error) => flow_source_error_response(error),
        }
    }

    pub fn get_flow(&self, name: &str) -> RuntimeRouteResponse {
        let flow_path = match resolve_flow_path(&self.settings.flows_dir, name) {
            Ok(path) => path,
            Err(error) => return flow_source_error_response(error),
        };
        if !flow_path.exists() {
            return RuntimeRouteResponse::json(404, json!({"detail": "Flow not found."}));
        }
        let content = match fs::read_to_string(&flow_path) {
            Ok(content) => content,
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
            }
        };
        let name = match flow_name_from_path(&self.settings.flows_dir, &flow_path) {
            Ok(name) => name,
            Err(error) => return flow_source_error_response(error),
        };
        RuntimeRouteResponse::json(200, json!({"name": name, "content": content}))
    }

    pub fn save_flow(&self, req: SaveFlowRequest) -> RuntimeRouteResponse {
        save_flow_request(&self.settings.flows_dir, req)
    }

    pub fn delete_flow(&self, name: &str) -> RuntimeRouteResponse {
        let flow_path = match resolve_flow_path(&self.settings.flows_dir, name) {
            Ok(path) => path,
            Err(error) => return flow_source_error_response(error),
        };
        if !flow_path.exists() {
            return RuntimeRouteResponse::json(404, json!({"detail": "Flow not found."}));
        }
        match fs::remove_file(&flow_path) {
            Ok(()) => RuntimeRouteResponse::json(200, json!({"status": "deleted"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn list_llm_profiles(&self) -> RuntimeRouteResponse {
        match unified_llm_adapter::public_llm_profiles(&self.settings.config_dir) {
            Ok(profiles) => RuntimeRouteResponse::json(200, json!({"profiles": profiles})),
            Err(error) => RuntimeRouteResponse::json(400, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_execution_placement_settings(&self) -> RuntimeRouteResponse {
        execution_placement_settings(&self.settings)
    }

    pub fn start_pipeline(&self, request: PipelineStartRequest) -> RuntimeRouteResponse {
        let store = self.observed_store();
        let wait_for_terminal = request.wait.unwrap_or(false);
        let flow_name = trimmed_option(request.flow_name.as_deref());
        let mut flow_content = trimmed_option(request.flow_content.as_deref());
        let mut flow_source_dir = None;
        if flow_content.is_none() {
            let Some(flow_name) = flow_name.as_deref() else {
                return validation_error_response("Either flow_content or flow_name is required.");
            };
            match load_flow_content(&self.settings.flows_dir, flow_name) {
                Ok(content) => flow_content = Some(content),
                Err(error) => {
                    return RuntimeRouteResponse::json(
                        200,
                        json!({
                            "status": if error.status_code() == 400 { "validation_error" } else { "failed" },
                            "error": error.detail(),
                        }),
                    );
                }
            }
        }
        if let Some(flow_name) = flow_name.as_deref() {
            if let Ok(flow_path) = resolve_flow_path(&self.settings.flows_dir, flow_name) {
                if flow_path.exists() {
                    flow_source_dir = flow_path.parent().map(Path::to_path_buf);
                }
            }
        }

        let mut flow_content = flow_content.unwrap_or_default();
        if let Some(goal) = trimmed_option(request.goal.as_deref()) {
            flow_content = match inject_pipeline_goal(&flow_content, &goal) {
                Ok(content) => content,
                Err(error) => return pipeline_parse_validation_response(error),
            };
        }

        let graph = match parse_dot(&flow_content) {
            Ok(graph) => graph,
            Err(error) => return pipeline_parse_validation_response(error),
        };
        let graph = apply_graph_transforms(&graph);
        let diagnostics = validate_graph(&graph);
        let errors = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .cloned()
            .collect::<Vec<_>>();
        let diagnostic_payloads = diagnostics_payload(&diagnostics);
        let error_payloads = diagnostics_payload(&errors);
        if !errors.is_empty() {
            return RuntimeRouteResponse::json(
                200,
                json!({
                    "status": "validation_error",
                    "diagnostics": diagnostic_payloads,
                    "errors": error_payloads,
                }),
            );
        }

        let requested_context = request.launch_context.unwrap_or_default();
        let launch_context = match LaunchContext::new(requested_context.clone()) {
            Ok(launch_context) => launch_context,
            Err(error) => return validation_error_response(error.to_string()),
        };
        let _start_node = match attractor_runtime::resolve_start_node(&graph) {
            Ok(start_node) => start_node,
            Err(error) => return validation_error_response(error.to_string()),
        };

        let run_id = self.reserve_run_id(&store, request.run_id.as_deref());
        let run_id = match run_id {
            Ok(run_id) => run_id,
            Err(error) => return validation_error_response(error),
        };
        let working_directory = absolutize_path(&request.working_directory);
        let (selected_model, display_model) =
            resolve_launch_model(&graph, request.model.as_deref(), &requested_context);
        let selected_provider =
            resolve_launch_provider(&graph, request.llm_provider.as_deref(), &requested_context)
                .unwrap_or_else(|| "codex".to_string());
        let selected_profile =
            resolve_launch_profile(&graph, request.llm_profile.as_deref(), &requested_context);
        let selected_reasoning_effort = resolve_launch_reasoning_effort(
            request.reasoning_effort.as_deref(),
            &requested_context,
        );
        let execution_selection = match attractor_execution::resolve_execution_profile_by_id(
            &self.settings,
            request.execution_profile_id.as_deref(),
            request.project_default_execution_profile_id.as_deref(),
            None,
        ) {
            Ok(selection) => selection,
            Err(error) => {
                return RuntimeRouteResponse::json(
                    200,
                    json!({
                        "status": "validation_error",
                        "error": error.to_string(),
                        "diagnostics": diagnostic_payloads,
                        "errors": error_payloads,
                    }),
                );
            }
        };
        let execution_metadata = attractor_execution::build_launch_metadata(&execution_selection);

        let mut record = RunRecord::new(&run_id, &working_directory);
        record.flow_name = flow_name.clone().unwrap_or_default();
        record.working_directory = working_directory.clone();
        record.project_path = working_directory.clone();
        record.model = display_model.clone();
        record.provider = selected_provider.clone();
        record.llm_provider = selected_provider.clone();
        record.llm_profile = selected_profile.clone();
        record.reasoning_effort = selected_reasoning_effort.clone();
        record.spec_id = trimmed_option(request.spec_id.as_deref());
        record.plan_id = trimmed_option(request.plan_id.as_deref());
        record.root_run_id = Some(run_id.clone());
        attractor_execution::apply_launch_metadata_to_record(&mut record, &execution_metadata);

        let mut runtime_context = requested_context;
        runtime_context.insert(
            unified_llm_adapter::RUNTIME_LAUNCH_MODEL_KEY.to_string(),
            json!(selected_model.clone().unwrap_or_default()),
        );
        runtime_context.insert(
            unified_llm_adapter::RUNTIME_LAUNCH_PROVIDER_KEY.to_string(),
            json!(selected_provider.clone()),
        );
        runtime_context.insert(
            unified_llm_adapter::RUNTIME_LAUNCH_PROFILE_KEY.to_string(),
            json!(selected_profile.clone().unwrap_or_default()),
        );
        runtime_context.insert(
            unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
            json!(selected_reasoning_effort.clone().unwrap_or_default()),
        );
        runtime_context.extend(execution_metadata.as_context_updates());
        runtime_context.insert("internal.run_id".to_string(), json!(run_id.clone()));
        runtime_context.insert("internal.root_run_id".to_string(), json!(run_id.clone()));
        runtime_context.insert(
            "internal.run_workdir".to_string(),
            json!(working_directory.clone()),
        );
        if let Some(flow_source_dir) = flow_source_dir.as_ref() {
            runtime_context.insert(
                "internal.flow_source_dir".to_string(),
                json!(flow_source_dir.to_string_lossy().to_string()),
            );
        }

        if let Err(error) = fs::create_dir_all(&working_directory) {
            return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
        }

        let graph_dot = format_readable_dot(&graph);
        // Create the run on disk before responding: the run_id resolves to a
        // real record (and initial journal) the moment the caller sees it.
        let paths = match attractor_runtime::prepare_fresh_run(
            &store,
            &record,
            &graph,
            Some(flow_content.clone()),
            Some(graph_dot),
            &launch_context,
            &runtime_context,
        ) {
            Ok(paths) => paths,
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
            }
        };
        let execute_request = ExecuteRunRequest {
            store: store.clone(),
            record,
            graph: graph.clone(),
            graph_source: None,
            graph_dot: None,
            launch_context,
            runtime_context,
            max_steps: None,
            start: ExecutionStart::Prepared {
                paths: paths.clone(),
            },
        };

        let terminal_status = if wait_for_terminal {
            let mut executor = PipelineExecutor::with_control(
                self.observed_node_executor(),
                attractor_runtime::disk_execution_control(paths.clone()),
            );
            match executor.execute(execute_request) {
                Ok(result) => result.status,
                Err(error) => {
                    return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
                }
            }
        } else {
            if let Err(error) = self.spawn_detached_execution(paths.clone(), execute_request) {
                let _ = store.update_run_record(&run_id, |record| {
                    record.status = "failed".to_string();
                    record.last_error = error.clone();
                });
                return RuntimeRouteResponse::json(500, json!({"detail": error}));
            }
            "running".to_string()
        };

        let graph_paths = Some(graph_artifact_paths(&paths));
        let execution_metadata_value =
            serde_json::to_value(&execution_metadata).unwrap_or_else(|_| json!({}));
        RuntimeRouteResponse::json(
            200,
            start_response_payload(
                &run_id,
                &working_directory,
                &display_model,
                &selected_provider,
                selected_profile.as_deref(),
                selected_reasoning_effort.as_deref(),
                execution_metadata_value,
                diagnostic_payloads,
                error_payloads,
                graph_paths,
                terminal_status,
            ),
        )
    }

    pub fn get_pipeline(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => {
                let children = store
                    .list_child_run_bundles(pipeline_id)
                    .unwrap_or_else(|_| Vec::new());
                RuntimeRouteResponse::json(200, pipeline_detail_payload(&bundle, &children))
            }
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_pipeline_checkpoint(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        RuntimeControlService::new(RunStore::for_settings(&self.settings))
            .get_checkpoint(pipeline_id)
    }

    pub fn get_pipeline_context(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let controls = RuntimeControls::new(RunStore::for_settings(&self.settings));
        match controls.get_context(pipeline_id) {
            Ok(context) => RuntimeRouteResponse::json(
                200,
                json!({"pipeline_id": pipeline_id, "context": context}),
            ),
            Err(error) => runtime_error_response(error, "Context unavailable"),
        }
    }

    pub fn get_pipeline_result(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let status = bundle
            .record
            .as_ref()
            .map(|record| attractor_runtime::normalize_run_status(&record.status))
            .unwrap_or_else(|| "unknown".to_string());
        if matches!(
            status.as_str(),
            "queued" | "running" | "pause_requested" | "cancel_requested"
        ) {
            return RuntimeRouteResponse::json(
                200,
                json!(attractor_core::RunResult::pending(pipeline_id, status)),
            );
        }
        match store.read_result(&bundle.paths) {
            Ok(Some(result)) if result.state != "pending" || !is_terminal_status(&status) => {
                RuntimeRouteResponse::json(200, json!(result))
            }
            Ok(_) if is_terminal_status(&status) => RuntimeRouteResponse::json(
                200,
                json!(attractor_core::RunResult::unavailable(pipeline_id, status)),
            ),
            Ok(Some(result)) => RuntimeRouteResponse::json(200, json!(result)),
            Ok(None) => RuntimeRouteResponse::json(
                200,
                json!(attractor_core::RunResult::pending(pipeline_id, status)),
            ),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn list_pipeline_artifacts(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        match store.list_artifacts(&bundle.paths) {
            Ok(artifacts) => RuntimeRouteResponse::json(
                200,
                json!({"pipeline_id": pipeline_id, "artifacts": artifacts}),
            ),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_pipeline_artifact_file(
        &self,
        pipeline_id: &str,
        artifact_path: &str,
    ) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        match store.read_artifact(&bundle.paths, artifact_path) {
            Ok(Some(file)) => {
                let content_type = artifact_media_type(&file.absolute_path);
                match String::from_utf8(file.content) {
                    Ok(body) => RuntimeRouteResponse::file(200, content_type, body),
                    Err(_) => RuntimeRouteResponse::json(
                        415,
                        json!({"detail": "Artifact is not viewable as text"}),
                    ),
                }
            }
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Artifact not found"})),
            Err(attractor_runtime::RuntimeStorageError::UnsafeArtifactPath { .. }) => {
                RuntimeRouteResponse::json(400, json!({"detail": "Invalid artifact path"}))
            }
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn get_pipeline_journal(
        &self,
        pipeline_id: &str,
        limit: Option<i64>,
        before_sequence: Option<i64>,
    ) -> RuntimeRouteResponse {
        let limit = match normalize_journal_limit(limit) {
            Ok(limit) => limit,
            Err(detail) => return RuntimeRouteResponse::json(400, json!({"detail": detail})),
        };
        let before_sequence = match normalize_before_sequence(before_sequence) {
            Ok(value) => value,
            Err(detail) => return RuntimeRouteResponse::json(400, json!({"detail": detail})),
        };
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let mut entries = bundle.journal;
        if let Some(before_sequence) = before_sequence {
            entries.retain(|entry| entry.sequence < before_sequence);
        }
        let has_older = entries.len() > limit;
        entries.truncate(limit);
        let oldest_sequence = entries.last().map(|entry| entry.sequence);
        let newest_sequence = entries.first().map(|entry| entry.sequence);
        RuntimeRouteResponse::json(
            200,
            json!({
                "pipeline_id": pipeline_id,
                "entries": entries,
                "oldest_sequence": oldest_sequence,
                "newest_sequence": newest_sequence,
                "has_older": has_older,
            }),
        )
    }

    pub fn get_pipeline_events(
        &self,
        pipeline_id: &str,
        after_sequence: Option<i64>,
    ) -> RuntimeRouteResponse {
        let after_sequence = match normalize_after_sequence(after_sequence) {
            Ok(value) => value,
            Err(detail) => return RuntimeRouteResponse::json(400, json!({"detail": detail})),
        };
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let Some(after_sequence) = after_sequence else {
            return RuntimeRouteResponse::event_stream(200, "");
        };
        let mut entries = bundle
            .journal
            .into_iter()
            .filter(|entry| entry.sequence > after_sequence)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.sequence);
        let mut stream = String::new();
        for entry in entries {
            let payload = serde_json::to_string(&entry)
                .expect("serializing journal entry for SSE cannot fail");
            stream.push_str("data: ");
            stream.push_str(&payload);
            stream.push_str("\n\n");
        }
        RuntimeRouteResponse::event_stream(200, stream)
    }

    pub fn get_pipeline_graph(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        self.get_pipeline_artifact_file(pipeline_id, "artifacts/graphviz/pipeline.svg")
            .with_not_found_detail("Graph visualization unavailable")
            .with_content_type("image/svg+xml")
    }

    pub fn get_pipeline_graph_preview(
        &self,
        pipeline_id: &str,
        expand_children: bool,
    ) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let source = match store.read_graph_source(&bundle.paths) {
            Ok(Some(source)) => source,
            Ok(None) => {
                return RuntimeRouteResponse::json(
                    404,
                    json!({"detail": "Run graph preview unavailable"}),
                );
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let (flow_source_dir, run_workdir) = preview_source_context(&bundle);
        RuntimeRouteResponse::json(
            200,
            preview_response_payload_with_options(
                &source,
                PreviewOptions {
                    expand_children,
                    flow_source_dir,
                    run_workdir,
                },
            ),
        )
    }

    pub fn continue_pipeline_route(
        &self,
        pipeline_id: &str,
        request: ContinuePipelineRequest,
    ) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let source = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        if source.checkpoint.is_none() {
            return RuntimeRouteResponse::json(404, json!({"detail": "Checkpoint unavailable"}));
        }
        let source_record = source.record.as_ref();
        let mode = request.flow_source_mode.trim().to_lowercase();
        let (flow_name, flow_content) = if mode == "snapshot" {
            let flow_name = source_record
                .map(|record| record.flow_name.clone())
                .unwrap_or_default();
            match store.read_graph_source(&source.paths) {
                Ok(Some(source)) => (flow_name, source),
                Ok(None) => {
                    return RuntimeRouteResponse::json(
                        200,
                        json!({"status": "failed", "error": "Run graph preview unavailable"}),
                    );
                }
                Err(error) => {
                    return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
                }
            }
        } else if mode == "flow_name" {
            let Some(flow_name) = trimmed_option(request.flow_name.as_deref()) else {
                return RuntimeRouteResponse::json(
                    200,
                    json!({"status": "validation_error", "error": "flow_name is required when flow_source_mode is flow_name."}),
                );
            };
            match load_flow_content(&self.settings.flows_dir, &flow_name) {
                Ok(content) => (flow_name, content),
                Err(error) => {
                    return RuntimeRouteResponse::json(
                        200,
                        json!({
                            "status": if error.status_code() == 400 { "validation_error" } else { "failed" },
                            "error": error.detail(),
                        }),
                    );
                }
            }
        } else {
            return RuntimeRouteResponse::json(
                200,
                json!({"status": "validation_error", "error": "flow_source_mode must be either snapshot or flow_name."}),
            );
        };
        let graph = match parse_dot(&flow_content) {
            Ok(graph) => apply_graph_transforms(&graph),
            Err(error) => return pipeline_parse_validation_response(error),
        };
        let response = RuntimeControlService::new(self.observed_store()).continue_pipeline(
            pipeline_id,
            request,
            ResolvedContinueFlow {
                graph: graph.clone(),
                graph_source: Some(flow_content.clone()),
                graph_dot: Some(flow_content),
                flow_name: Some(flow_name),
            },
        );
        self.execute_prepared_route_response(&response, graph);
        response
    }

    pub fn retry_pipeline_route(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        let store = self.observed_store();
        let response = RuntimeControlService::new(store.clone()).retry_pipeline(pipeline_id);
        if response.status_code < 400
            && response.body.get("status").and_then(Value::as_str) == Some("started")
        {
            // Retry re-executes the same run from its adjusted checkpoint; the
            // graph comes from the run's stored snapshot source.
            let graph = store
                .read_run_bundle(pipeline_id)
                .ok()
                .flatten()
                .and_then(|bundle| store.read_graph_source(&bundle.paths).ok().flatten())
                .and_then(|source| parse_dot(&source).ok())
                .map(|graph| apply_graph_transforms(&graph));
            match graph {
                Some(graph) => self.execute_prepared_route_response(&response, graph),
                None => {
                    let _ = store.update_run_record(pipeline_id, |record| {
                        record.status = "failed".to_string();
                        record.last_error =
                            "Retry could not load the stored run graph source.".to_string();
                    });
                }
            }
        }
        response
    }

    pub fn cancel_pipeline_route(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        RuntimeControlService::new(RunStore::for_settings(&self.settings))
            .cancel_pipeline(pipeline_id)
    }

    pub fn steer_pipeline_route(
        &self,
        pipeline_id: &str,
        request: PipelineSteerRequest,
    ) -> RuntimeRouteResponse {
        let message = request.message.trim();
        if message.is_empty() {
            return validation_error_response("message is required.");
        }
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        let (target_run_id, target_node_id) = intervention_target(&bundle, &request);
        let result = json!({
            "run_id": target_run_id,
            "status": "rejected",
            "delivery_mode": "none",
            "reason": "no_active_child_run",
            "message": "No active child run is available for intervention.",
            "target_node_id": target_node_id,
        });
        if let Err(error) = store.append_event(
            &bundle.paths,
            human_intervention_requested_event(
                pipeline_id,
                &target_run_id,
                target_node_id.clone(),
                message,
                "rejected",
                "none",
                "no_active_child_run",
                result.clone(),
            ),
        ) {
            return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
        }
        let mut payload = serde_json::Map::new();
        payload.insert("pipeline_id".to_string(), json!(pipeline_id));
        payload.insert("target_run_id".to_string(), json!(target_run_id));
        if let Some(object) = result.as_object() {
            payload.extend(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        RuntimeRouteResponse::json(200, Value::Object(payload))
    }

    pub fn patch_pipeline_metadata(
        &self,
        pipeline_id: &str,
        request: PipelineMetadataUpdateRequest,
    ) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let updated = match store.update_run_record(pipeline_id, |record| {
            if let Some(spec_id) = trimmed_option(request.spec_id.as_deref()) {
                record.spec_id = Some(spec_id);
            }
            if let Some(plan_id) = trimmed_option(request.plan_id.as_deref()) {
                record.plan_id = Some(plan_id);
            }
        }) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
            }
        };
        if let Ok(Some(bundle)) = store.read_run_bundle(pipeline_id) {
            let _ = store.append_event(
                &bundle.paths,
                attractor_runtime::run_metadata_event_with_graph_paths(&updated, &bundle.paths),
            );
        }
        RuntimeRouteResponse::json(200, json!(updated))
    }

    pub fn list_pipeline_questions(&self, pipeline_id: &str) -> RuntimeRouteResponse {
        match RunStore::for_settings(&self.settings).read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => RuntimeRouteResponse::json(
                200,
                json!({"questions": pending_pipeline_questions(&bundle)}),
            ),
            Ok(None) => RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn answer_pipeline_question(
        &self,
        pipeline_id: &str,
        question_id: &str,
        request: HumanAnswerRequest,
    ) -> RuntimeRouteResponse {
        let store = RunStore::for_settings(&self.settings);
        let bundle = match store.read_run_bundle(pipeline_id) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}));
            }
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
            }
        };
        let question = pending_pipeline_questions(&bundle)
            .into_iter()
            .find(|question| {
                question.get("question_id").and_then(Value::as_str) == Some(question_id)
            });
        let Some(question) = question else {
            return RuntimeRouteResponse::json(
                404,
                json!({"detail": "Unknown question for pipeline"}),
            );
        };
        let answer = request.selected_value.trim();
        if answer.is_empty() {
            return validation_error_response("selected_value is required.");
        }
        let event = attractor_runtime::human_gate_answered_event(
            pipeline_id,
            question_id,
            question
                .get("node_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            question
                .get("flow_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            question
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_string),
            answer.to_string(),
        );
        if let Err(error) = store.append_event(&bundle.paths, event) {
            return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
        }
        RuntimeRouteResponse::json(
            200,
            json!({"status": "accepted", "pipeline_id": pipeline_id, "question_id": question_id}),
        )
    }

    pub fn reset(&self) -> RuntimeRouteResponse {
        match RunStore::for_settings(&self.settings).delete_all_runs() {
            Ok(()) => RuntimeRouteResponse::json(200, json!({"status": "reset"})),
            Err(error) => RuntimeRouteResponse::json(500, json!({"detail": error.to_string()})),
        }
    }

    pub fn dispatch(&self, method: &str, path: &str, body: &str) -> RuntimeRouteResponse {
        dispatch_attractor_route(self, method, path, body)
    }

    fn reserve_run_id(&self, store: &RunStore, requested: Option<&str>) -> Result<String, String> {
        if let Some(requested) = trimmed_option(requested) {
            return match store.find_run_root(&requested) {
                Ok(Some(_)) => Err(format!("Run id already exists: {requested}")),
                Ok(None) => Ok(requested),
                Err(error) => Err(error.to_string()),
            };
        }
        for _ in 0..100 {
            let run_id = generated_run_id();
            match store.find_run_root(&run_id) {
                Ok(Some(_)) => continue,
                Ok(None) => return Ok(run_id),
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Unable to allocate run id".to_string())
    }
}

pub fn handle_attractor_request(
    method: &str,
    path: &str,
    body: &str,
    settings: SparkSettings,
) -> RuntimeRouteResponse {
    AttractorApiService::new(settings).dispatch(method, path, body)
}

pub fn handle_attractor_request_with_runtime_handler_runner_factory(
    method: &str,
    path: &str,
    body: &str,
    settings: SparkSettings,
    runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
) -> RuntimeRouteResponse {
    handle_attractor_request_with_options(
        method,
        path,
        body,
        settings,
        runtime_handler_runner_factory,
        None,
    )
}

pub fn handle_attractor_request_with_options(
    method: &str,
    path: &str,
    body: &str,
    settings: SparkSettings,
    runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
) -> RuntimeRouteResponse {
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings,
        runtime_handler_runner_factory,
    );
    let service = match run_event_observer {
        Some(observer) => service.with_run_event_observer(observer),
        None => service,
    };
    service.dispatch(method, path, body)
}

pub fn default_runtime_handler_runner_factory() -> RuntimeHandlerRunnerFactory {
    Arc::new(RuntimeHandlerRunner::new)
}

pub fn rust_llm_runtime_handler_runner_factory(
    client: unified_llm_adapter::Client,
) -> RuntimeHandlerRunnerFactory {
    Arc::new(move || RuntimeHandlerRunner::new().with_rust_llm_client(client.clone()))
}

pub fn preview(req: PreviewRequest) -> PreviewRouteResponse {
    preview_with_config(req, &PreviewServiceConfig::default())
}

pub fn preview_with_flows_dir(
    req: PreviewRequest,
    flows_dir: impl AsRef<Path>,
) -> PreviewRouteResponse {
    preview_with_config(
        req,
        &PreviewServiceConfig {
            flows_dir: Some(flows_dir.as_ref().to_path_buf()),
        },
    )
}

pub fn preview_with_config(
    req: PreviewRequest,
    config: &PreviewServiceConfig,
) -> PreviewRouteResponse {
    let flow_source_dir = resolve_preview_flow_source_dir(req.flow_name.as_deref(), config);
    let body = preview_response_payload_with_options(
        &req.flow_content,
        PreviewOptions {
            expand_children: req.expand_children,
            flow_source_dir,
            run_workdir: None,
        },
    );
    PreviewRouteResponse::json(200, body)
}

pub fn list_logical_flow_names(
    flows_dir: impl AsRef<Path>,
) -> Result<Vec<String>, FlowSourceError> {
    list_flow_names(flows_dir.as_ref())
}

pub fn resolve_logical_flow_path(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<PathBuf, FlowSourceError> {
    resolve_flow_path(flows_dir, flow_name)
}

pub fn read_named_flow_source(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<NamedFlowSource, FlowSourceError> {
    let flows_dir = flows_dir.as_ref();
    let path = resolve_flow_path(flows_dir, flow_name)?;
    if !path.exists() {
        return Err(FlowSourceError::new(404, "Flow not found."));
    }
    let content = fs::read_to_string(&path).map_err(|source| {
        FlowSourceError::new(
            500,
            format!("Unable to read flow file {}: {source}", path.display()),
        )
    })?;
    let name = flow_name_from_path(flows_dir, &path)?;
    Ok(NamedFlowSource {
        name,
        path,
        content,
    })
}

pub fn preview_named_flow_source(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
    flow_content: &str,
) -> PreviewRouteResponse {
    preview_with_flows_dir(
        PreviewRequest {
            flow_content: flow_content.to_string(),
            flow_name: Some(flow_name.to_string()),
            expand_children: false,
        },
        flows_dir,
    )
}

pub fn handle_preview_request(
    method: &str,
    path: &str,
    body: &str,
    config: &PreviewServiceConfig,
) -> PreviewRouteResponse {
    if method != "POST" || normalize_attractor_path(path) != "/preview" {
        return PreviewRouteResponse::json(404, json!({"detail": "Not Found"}));
    }
    let req = match serde_json::from_str::<PreviewRequest>(body) {
        Ok(req) => req,
        Err(error) => {
            return PreviewRouteResponse::json(
                400,
                json!({"detail": format!("Invalid preview request JSON: {error}")}),
            );
        }
    };
    preview_with_config(req, config)
}

fn dispatch_attractor_route(
    service: &AttractorApiService,
    method: &str,
    path: &str,
    body: &str,
) -> RuntimeRouteResponse {
    let method = method.to_ascii_uppercase();
    let (path, query) = split_attractor_path_query(path);
    match (method.as_str(), path.as_str()) {
        ("GET", "/status") => service.get_status(),
        ("GET", "/runs") => {
            service.list_runs_for_project(query_string(&query, "project_path").as_deref())
        }
        ("GET", "/runs/events") => service.deprecated_runs_events(),
        ("POST", "/pipelines") => match serde_json::from_str::<PipelineStartRequest>(body) {
            Ok(req) => service.start_pipeline(req),
            Err(error) => RuntimeRouteResponse::json(
                400,
                json!({"detail": format!("Invalid pipeline start request JSON: {error}")}),
            ),
        },
        ("POST", "/reset") => service.reset(),
        ("POST", "/preview") => match serde_json::from_str::<PreviewRequest>(body) {
            Ok(req) => service.preview(req),
            Err(error) => RuntimeRouteResponse::json(
                400,
                json!({"detail": format!("Invalid preview request JSON: {error}")}),
            ),
        },
        ("GET", "/api/flows") => service.list_flows(),
        ("POST", "/api/flows") => match serde_json::from_str::<SaveFlowRequest>(body) {
            Ok(req) => service.save_flow(req),
            Err(error) => RuntimeRouteResponse::json(
                400,
                json!({"detail": format!("Invalid flow save request JSON: {error}")}),
            ),
        },
        ("GET", "/api/llm-profiles") => service.list_llm_profiles(),
        ("GET", "/api/execution-placement-settings") => service.get_execution_placement_settings(),
        ("GET", path) if path.starts_with("/api/flows/") => {
            service.get_flow(path.trim_start_matches("/api/flows/"))
        }
        ("DELETE", path) if path.starts_with("/api/flows/") => {
            service.delete_flow(path.trim_start_matches("/api/flows/"))
        }
        _ if path.starts_with("/pipelines/") => {
            dispatch_pipeline_route(service, &method, &path, &query, body)
        }
        _ => RuntimeRouteResponse::json(404, json!({"detail": "Not Found"})),
    }
}

fn normalize_attractor_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let path = if path == "/attractor" {
        "/"
    } else if let Some(rest) = path.strip_prefix("/attractor/") {
        return format!("/{rest}");
    } else {
        path
    };
    path.to_string()
}

fn split_attractor_path_query(path: &str) -> (String, String) {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    (normalize_attractor_path(path), query.to_string())
}

fn dispatch_pipeline_route(
    service: &AttractorApiService,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
) -> RuntimeRouteResponse {
    let remainder = path.trim_start_matches("/pipelines/");
    let (pipeline_id, subpath) = remainder.split_once('/').unwrap_or((remainder, ""));
    if pipeline_id.trim().is_empty() {
        return RuntimeRouteResponse::json(404, json!({"detail": "Not Found"}));
    }
    match (method, subpath) {
        ("GET", "") => service.get_pipeline(pipeline_id),
        ("GET", "checkpoint") => service.get_pipeline_checkpoint(pipeline_id),
        ("GET", "context") => service.get_pipeline_context(pipeline_id),
        ("GET", "result") => service.get_pipeline_result(pipeline_id),
        ("GET", "artifacts") => service.list_pipeline_artifacts(pipeline_id),
        ("GET", subpath) if subpath.starts_with("artifacts/") => {
            match percent_decode_path(subpath.strip_prefix("artifacts/").unwrap_or(subpath)) {
                Ok(artifact_path) => {
                    service.get_pipeline_artifact_file(pipeline_id, &artifact_path)
                }
                Err(()) => {
                    RuntimeRouteResponse::json(400, json!({"detail": "Invalid artifact path"}))
                }
            }
        }
        ("GET", "journal") => service.get_pipeline_journal(
            pipeline_id,
            query_i64(query, "limit"),
            query_i64(query, "before_sequence"),
        ),
        ("GET", "events") => match query_i64_strict(
            query,
            "after_sequence",
            "after_sequence must be zero or greater",
        ) {
            Ok(after_sequence) => service.get_pipeline_events(pipeline_id, after_sequence),
            Err(detail) => RuntimeRouteResponse::json(400, json!({"detail": detail})),
        },
        ("GET", "graph") => service.get_pipeline_graph(pipeline_id),
        ("GET", "graph-preview") => service.get_pipeline_graph_preview(
            pipeline_id,
            query_bool(query, "expand_children").unwrap_or(false),
        ),
        ("GET", "questions") => service.list_pipeline_questions(pipeline_id),
        ("POST", "continue") => match serde_json::from_str::<ContinuePipelineRequest>(body) {
            Ok(req) => service.continue_pipeline_route(pipeline_id, req),
            Err(error) => RuntimeRouteResponse::json(
                400,
                json!({"detail": format!("Invalid pipeline continue request JSON: {error}")}),
            ),
        },
        ("POST", "retry") => service.retry_pipeline_route(pipeline_id),
        ("POST", "cancel") => service.cancel_pipeline_route(pipeline_id),
        ("POST", "steer") => match serde_json::from_str::<PipelineSteerRequest>(body) {
            Ok(req) => service.steer_pipeline_route(pipeline_id, req),
            Err(error) => RuntimeRouteResponse::json(
                400,
                json!({"detail": format!("Invalid pipeline steer request JSON: {error}")}),
            ),
        },
        ("PATCH", "metadata") => {
            match serde_json::from_str::<PipelineMetadataUpdateRequest>(body) {
                Ok(req) => service.patch_pipeline_metadata(pipeline_id, req),
                Err(error) => RuntimeRouteResponse::json(
                    400,
                    json!({"detail": format!("Invalid pipeline metadata request JSON: {error}")}),
                ),
            }
        }
        ("POST", subpath) if subpath.starts_with("questions/") && subpath.ends_with("/answer") => {
            let question_id = subpath
                .trim_start_matches("questions/")
                .trim_end_matches("/answer");
            match serde_json::from_str::<HumanAnswerRequest>(body) {
                Ok(req) => service.answer_pipeline_question(pipeline_id, question_id, req),
                Err(error) => RuntimeRouteResponse::json(
                    400,
                    json!({"detail": format!("Invalid human answer request JSON: {error}")}),
                ),
            }
        }
        _ => RuntimeRouteResponse::json(404, json!({"detail": "Not Found"})),
    }
}

fn list_flow_names(flows_dir: &Path) -> Result<Vec<String>, FlowSourceError> {
    let flows_dir = ensure_flows_dir(flows_dir)?;
    let mut paths = Vec::new();
    collect_dot_paths(&flows_dir, &mut paths).map_err(|source| {
        FlowSourceError::new(
            500,
            format!(
                "Unable to list flows directory {}: {source}",
                flows_dir.display()
            ),
        )
    })?;
    let mut names = paths
        .into_iter()
        .map(|path| flow_name_from_path(&flows_dir, path))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn collect_dot_paths(dir: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_dot_paths(&path, paths)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("dot") {
            paths.push(path);
        }
    }
    Ok(())
}

fn save_flow_request(flows_dir: &Path, req: SaveFlowRequest) -> RuntimeRouteResponse {
    let graph = match parse_dot(&req.content) {
        Ok(graph) => graph,
        Err(error) => return flow_save_parse_error_response(error),
    };
    let canonical_content = format_readable_dot(&graph);
    let transformed_graph = apply_graph_transforms(&graph);
    let diagnostics = validate_graph(&transformed_graph);
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .cloned()
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return flow_save_validation_error_response(&diagnostics, &errors);
    }

    let flow_path = match resolve_flow_path(flows_dir, &req.name) {
        Ok(path) => path,
        Err(error) => return flow_source_error_response(error),
    };
    let semantic_equivalent_to_existing = if flow_path.exists() {
        match fs::read_to_string(&flow_path) {
            Ok(existing_content) => match (
                semantic_signature_for_source(&existing_content),
                semantic_signature_for_source(&req.content),
            ) {
                (Ok(left), Ok(right)) => Some(left == right),
                _ => None,
            },
            Err(error) => {
                return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
            }
        }
    } else {
        None
    };

    if req.expect_semantic_equivalence && semantic_equivalent_to_existing == Some(false) {
        return RuntimeRouteResponse::json(
            409,
            json!({
                "detail": {
                    "status": "semantic_mismatch",
                    "error": "semantic equivalence check failed: output DOT would change flow behavior",
                }
            }),
        );
    }

    if let Some(parent) = flow_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
        }
    }
    if let Err(error) = fs::write(&flow_path, canonical_content) {
        return RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}));
    }
    let name = match flow_name_from_path(flows_dir, &flow_path) {
        Ok(name) => name,
        Err(error) => return flow_source_error_response(error),
    };
    let mut response = serde_json::Map::from_iter([
        ("status".to_string(), json!("saved")),
        ("name".to_string(), json!(name)),
    ]);
    if let Some(semantic_equivalent_to_existing) = semantic_equivalent_to_existing {
        response.insert(
            "semantic_equivalent_to_existing".to_string(),
            json!(semantic_equivalent_to_existing),
        );
    }
    RuntimeRouteResponse::json(200, Value::Object(response))
}

fn flow_save_parse_error_response(error: DotParseError) -> RuntimeRouteResponse {
    let parse_diag = parse_error_diagnostic(&error);
    RuntimeRouteResponse::json(
        422,
        json!({
            "detail": {
                "status": "parse_error",
                "error": format!("invalid DOT: {error}"),
                "diagnostics": [parse_diag.clone()],
                "errors": [parse_diag],
            }
        }),
    )
}

fn flow_save_validation_error_response(
    diagnostics: &[Diagnostic],
    errors: &[Diagnostic],
) -> RuntimeRouteResponse {
    RuntimeRouteResponse::json(
        422,
        json!({
            "detail": {
                "status": "validation_error",
                "error": "validation errors prevent saving this flow",
                "diagnostics": diagnostics_payload(diagnostics),
                "errors": diagnostics_payload(errors),
            }
        }),
    )
}

fn parse_error_diagnostic(error: &DotParseError) -> Value {
    json!({
        "rule": "parse_error",
        "rule_id": "parse_error",
        "severity": "error",
        "message": error.to_string(),
        "line": error.line(),
        "node": null,
        "node_id": null,
    })
}

fn flow_source_error_response(error: FlowSourceError) -> RuntimeRouteResponse {
    RuntimeRouteResponse::json(error.status_code(), json!({"detail": error.detail()}))
}

fn default_working_directory() -> String {
    "./workspace".to_string()
}

fn validation_error_response(error: impl Into<String>) -> RuntimeRouteResponse {
    RuntimeRouteResponse::json(
        200,
        json!({"status": "validation_error", "error": error.into()}),
    )
}

fn pipeline_parse_validation_response(error: DotParseError) -> RuntimeRouteResponse {
    let parse_diag = parse_error_diagnostic(&error);
    RuntimeRouteResponse::json(
        200,
        json!({
            "status": "validation_error",
            "error": error.to_string(),
            "diagnostics": [parse_diag.clone()],
            "errors": [parse_diag],
        }),
    )
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn generated_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("run-{nanos:x}")
}

fn absolutize_path(value: &str) -> String {
    let raw = value.trim();
    let path = if raw.is_empty() {
        PathBuf::from(default_working_directory())
    } else {
        PathBuf::from(raw)
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    normalize_path_string(absolute)
}

fn normalize_path_string(path: PathBuf) -> String {
    path.components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .to_string()
}

fn resolve_launch_model(
    graph: &DotGraph,
    requested_model: Option<&str>,
    launch_context: &ContextMap,
) -> (Option<String>, String) {
    if let Some(model) = trimmed_real_model(requested_model) {
        return (Some(model.clone()), model);
    }
    if let Some(model) = trimmed_real_model(
        context_value_text(
            launch_context,
            unified_llm_adapter::RUNTIME_LAUNCH_MODEL_KEY,
        )
        .as_deref(),
    ) {
        return (Some(model.clone()), model);
    }
    if let Some(model) =
        trimmed_real_model(graph_attr_string(graph, "ui_default_llm_model").as_deref())
    {
        return (Some(model.clone()), model);
    }
    (
        None,
        unified_llm_adapter::DISPLAY_MODEL_PLACEHOLDER.to_string(),
    )
}

fn resolve_launch_provider(
    graph: &DotGraph,
    requested_provider: Option<&str>,
    launch_context: &ContextMap,
) -> Option<String> {
    trimmed_option(requested_provider)
        .or_else(|| {
            context_value_text(
                launch_context,
                unified_llm_adapter::RUNTIME_LAUNCH_PROVIDER_KEY,
            )
        })
        .or_else(|| graph_attr_string(graph, "ui_default_llm_provider"))
        .map(|provider| provider.to_lowercase())
}

fn resolve_launch_profile(
    graph: &DotGraph,
    requested_profile: Option<&str>,
    launch_context: &ContextMap,
) -> Option<String> {
    trimmed_option(requested_profile)
        .or_else(|| {
            context_value_text(
                launch_context,
                unified_llm_adapter::RUNTIME_LAUNCH_PROFILE_KEY,
            )
        })
        .or_else(|| graph_attr_string(graph, "ui_default_llm_profile"))
}

fn resolve_launch_reasoning_effort(
    requested: Option<&str>,
    launch_context: &ContextMap,
) -> Option<String> {
    trimmed_option(requested)
        .or_else(|| {
            context_value_text(
                launch_context,
                unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
            )
        })
        .map(|value| value.to_lowercase())
}

fn graph_attr_string(graph: &DotGraph, key: &str) -> Option<String> {
    graph
        .graph_attrs
        .get(key)
        .map(|attr| attr.value.to_string().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn context_value_text(context: &ContextMap, key: &str) -> Option<String> {
    let value = context.get(key)?;
    Some(match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok()?,
    })
    .and_then(|value| trimmed_option(Some(value.as_str())))
}

fn trimmed_real_model(value: Option<&str>) -> Option<String> {
    trimmed_option(value).filter(|value| !unified_llm_adapter::is_display_model_placeholder(value))
}

fn start_response_payload(
    run_id: &str,
    working_directory: &str,
    display_model: &str,
    provider: &str,
    llm_profile: Option<&str>,
    reasoning_effort: Option<&str>,
    execution_metadata: Value,
    diagnostics: Vec<Value>,
    errors: Vec<Value>,
    graph_paths: Option<Value>,
    terminal_status: String,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("status".to_string(), json!("started"));
    payload.insert("pipeline_id".to_string(), json!(run_id));
    payload.insert("run_id".to_string(), json!(run_id));
    payload.insert("working_directory".to_string(), json!(working_directory));
    payload.insert("model".to_string(), json!(display_model));
    payload.insert("provider".to_string(), json!(provider));
    payload.insert("llm_provider".to_string(), json!(provider));
    payload.insert("llm_profile".to_string(), option_str_json(llm_profile));
    payload.insert(
        "reasoning_effort".to_string(),
        option_str_json(reasoning_effort),
    );
    payload.insert("execution_lock".to_string(), Value::Null);
    if let Some(object) = execution_metadata.as_object() {
        payload.extend(
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    if !payload.contains_key("execution_container_image") {
        payload.insert("execution_container_image".to_string(), Value::Null);
    }
    payload.insert("diagnostics".to_string(), Value::Array(diagnostics));
    payload.insert("errors".to_string(), Value::Array(errors));
    if let Some(paths) = graph_paths.and_then(|value| value.as_object().cloned()) {
        payload.extend(paths);
    } else {
        payload.insert("graph_dot_path".to_string(), Value::Null);
        payload.insert("graph_render_path".to_string(), Value::Null);
    }
    payload.insert("terminal_status".to_string(), json!(terminal_status));
    Value::Object(payload)
}

fn option_str_json(value: Option<&str>) -> Value {
    value
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn graph_artifact_paths(paths: &attractor_runtime::RunRootPaths) -> Value {
    let graph_dir = paths.artifacts_dir().join("graphviz");
    let dot_path = graph_dir.join("pipeline.dot");
    let render_path = graph_dir.join("pipeline.svg");
    json!({
        "graph_dot_path": dot_path.to_string_lossy().to_string(),
        "graph_render_path": render_path.exists().then(|| render_path.to_string_lossy().to_string()),
    })
}

fn pipeline_detail_payload(bundle: &RunBundle, children: &[RunBundle]) -> Value {
    let mut payload = bundle
        .record
        .as_ref()
        .and_then(|record| serde_json::to_value(record).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_else(|| {
            serde_json::Map::from_iter([
                ("run_id".to_string(), json!(bundle.paths.run_id)),
                ("flow_name".to_string(), json!("")),
                ("status".to_string(), json!("unknown")),
                ("outcome".to_string(), Value::Null),
                ("outcome_reason_code".to_string(), Value::Null),
                ("outcome_reason_message".to_string(), Value::Null),
                ("working_directory".to_string(), json!("")),
                ("model".to_string(), json!("")),
                ("provider".to_string(), json!("codex")),
                ("llm_provider".to_string(), json!("codex")),
                ("llm_profile".to_string(), Value::Null),
                ("reasoning_effort".to_string(), Value::Null),
                ("started_at".to_string(), json!("")),
                ("ended_at".to_string(), Value::Null),
                ("last_error".to_string(), json!("")),
            ])
        });
    let current_node = bundle
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.current_node.clone());
    let completed_nodes = bundle
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.completed_nodes.clone())
        .unwrap_or_default();
    payload.insert("pipeline_id".to_string(), json!(bundle.paths.run_id));
    payload.insert(
        "run_id".to_string(),
        payload
            .get("run_id")
            .cloned()
            .filter(|value| !value.as_str().unwrap_or_default().is_empty())
            .unwrap_or_else(|| json!(bundle.paths.run_id)),
    );
    payload.insert("current_node".to_string(), json!(current_node));
    payload.insert("completed_nodes".to_string(), json!(completed_nodes));
    payload.insert(
        "progress".to_string(),
        json!({
            "current_node": payload.get("current_node").cloned().unwrap_or(Value::Null),
            "completed_nodes": payload.get("completed_nodes").cloned().unwrap_or_else(|| json!([])),
            "completed_count": payload
                .get("completed_nodes")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
        }),
    );
    payload.insert(
        "result_available".to_string(),
        json!(bundle.paths.result_json().exists() || bundle.paths.result_markdown().exists()),
    );
    payload.insert(
        "child_runs".to_string(),
        json!(child_run_summaries(children)),
    );
    Value::Object(payload)
}

fn pending_pipeline_questions(bundle: &RunBundle) -> Vec<Value> {
    let answered = answered_question_ids(&bundle.raw_events);
    bundle
        .raw_events
        .iter()
        .filter(|event| event.event_type == "human_gate")
        .filter(|event| matches!(event.payload.get("answer"), None | Some(Value::Null)))
        .filter_map(|event| pending_question_payload(event, &bundle.paths.run_id))
        .filter(|question| {
            question
                .get("question_id")
                .and_then(Value::as_str)
                .is_some_and(|question_id| !answered.iter().any(|answered| answered == question_id))
        })
        .collect()
}

fn answered_question_ids(events: &[RawRuntimeEvent]) -> Vec<String> {
    let mut question_ids = Vec::new();
    for event in events {
        let question_id = event_payload_string(event, "question_id");
        let Some(question_id) = question_id else {
            continue;
        };
        if event.event_type == "InterviewCompleted"
            || (event.event_type == "human_gate"
                && !matches!(event.payload.get("answer"), None | Some(Value::Null)))
        {
            question_ids.push(question_id);
        }
    }
    question_ids
}

fn pending_question_payload(event: &RawRuntimeEvent, pipeline_id: &str) -> Option<Value> {
    let question_id = event_payload_string(event, "question_id")?;
    let run_id = event_payload_string(event, "run_id").unwrap_or_else(|| event.run_id.clone());
    if run_id != pipeline_id {
        return None;
    }
    let node_id = event_payload_string(event, "node_id")
        .or_else(|| event_payload_string(event, "stage"))
        .unwrap_or_default();
    let flow_name = event_payload_string(event, "flow_name").unwrap_or_default();
    let prompt = event_payload_string(event, "prompt")
        .or_else(|| event_payload_string(event, "question"))
        .unwrap_or_default();
    let options = event
        .payload
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(json!({
        "question_id": question_id,
        "run_id": run_id,
        "node_id": node_id,
        "flow_name": flow_name,
        "prompt": prompt,
        "options": options,
    }))
}

fn event_payload_string(event: &RawRuntimeEvent, key: &str) -> Option<String> {
    event
        .payload
        .get(key)
        .and_then(value_to_string)
        .filter(|value| !value.is_empty())
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "validation_error" | "paused" | "canceled"
    )
}

fn artifact_media_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "md" => "text/markdown; charset=utf-8",
        "txt" | "log" | "dot" => "text/plain; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn normalize_journal_limit(limit: Option<i64>) -> Result<usize, String> {
    match limit {
        None => Ok(100),
        Some(value) if value <= 0 => Err("limit must be greater than zero".to_string()),
        Some(value) => Ok((value as usize).min(250)),
    }
}

fn normalize_before_sequence(value: Option<i64>) -> Result<Option<u64>, String> {
    match value {
        None => Ok(None),
        Some(value) if value <= 0 => Err("before_sequence must be greater than zero".to_string()),
        Some(value) => Ok(Some(value as u64)),
    }
}

fn normalize_after_sequence(value: Option<i64>) -> Result<Option<u64>, String> {
    match value {
        None => Ok(None),
        Some(value) if value < 0 => Err("after_sequence must be zero or greater".to_string()),
        Some(value) => Ok(Some(value as u64)),
    }
}

fn preview_source_context(bundle: &RunBundle) -> (Option<PathBuf>, Option<PathBuf>) {
    let context = bundle
        .checkpoint
        .as_ref()
        .map(|checkpoint| &checkpoint.context);
    let flow_source_dir = context
        .and_then(|context| context_path(context, "internal.flow_source_dir"))
        .map(PathBuf::from);
    let run_workdir = context
        .and_then(|context| context_path(context, "internal.run_workdir"))
        .map(PathBuf::from)
        .or_else(|| {
            bundle
                .record
                .as_ref()
                .and_then(|record| trimmed_option(Some(&record.working_directory)))
                .map(PathBuf::from)
        });
    (flow_source_dir, run_workdir)
}

fn context_path(context: &ContextMap, key: &str) -> Option<String> {
    context
        .get(key)
        .and_then(value_to_string)
        .filter(|value| !value.is_empty())
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.trim().to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn intervention_target(
    bundle: &RunBundle,
    request: &PipelineSteerRequest,
) -> (String, Option<String>) {
    if let Some(target_run_id) = trimmed_option(request.target_run_id.as_deref()) {
        return (
            target_run_id,
            trimmed_option(request.target_node_id.as_deref()),
        );
    }
    let context = bundle
        .checkpoint
        .as_ref()
        .map(|checkpoint| &checkpoint.context);
    let target_run_id = context
        .and_then(|context| context_path(context, "context.stack.child.run_id"))
        .unwrap_or_else(|| bundle.paths.run_id.clone());
    let target_node_id = trimmed_option(request.target_node_id.as_deref()).or_else(|| {
        context.and_then(|context| context_path(context, "context.stack.child.active_stage"))
    });
    (target_run_id, target_node_id)
}

fn query_i64(query: &str, key: &str) -> Option<i64> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then(|| value.parse::<i64>().ok()).flatten()
    })
}

fn query_i64_strict(query: &str, key: &str, invalid_detail: &str) -> Result<Option<i64>, String> {
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once('=') else {
            if part == key {
                return Err(invalid_detail.to_string());
            }
            continue;
        };
        if name != key {
            continue;
        }
        return value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| invalid_detail.to_string());
    }
    Ok(None)
}

fn query_bool(query: &str, key: &str) -> Option<bool> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        if name != key {
            return None;
        }
        Some(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ))
    })
}

fn query_string(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        if name != key {
            return None;
        }
        percent_decode_path(value).ok()
    })
}

fn percent_decode_path(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let high = hex_value(bytes[index + 1]).ok_or(())?;
            let low = hex_value(bytes[index + 2]).ok_or(())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn list_run_records(
    settings: &SparkSettings,
    project_path_filter: Option<&str>,
) -> Result<Vec<Value>, String> {
    let runs_dir = &settings.runs_dir;
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let project_path_filter = project_path_filter
        .and_then(|value| trimmed_option(Some(value)))
        .map(|value| absolutize_path(&value));
    let mut records = Vec::new();
    for project_entry in fs::read_dir(runs_dir).map_err(|error| error.to_string())? {
        let project_entry = project_entry.map_err(|error| error.to_string())?;
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }
        for run_entry in fs::read_dir(&project_path).map_err(|error| error.to_string())? {
            let run_entry = run_entry.map_err(|error| error.to_string())?;
            let run_path = run_entry.path();
            if !run_path.is_dir() {
                continue;
            }
            let record_path = run_path.join("run.json");
            if !record_path.exists() {
                if project_path_filter.is_some() {
                    continue;
                }
                records.push(json!({
                    "run_id": run_entry.file_name().to_string_lossy(),
                    "flow_name": "",
                    "status": "unknown",
                    "outcome": null,
                    "outcome_reason_code": null,
                    "outcome_reason_message": null,
                    "working_directory": "",
                    "model": "",
                    "llm_provider": "codex",
                    "reasoning_effort": null,
                    "started_at": "",
                }));
                continue;
            }
            let raw = fs::read_to_string(&record_path).map_err(|error| error.to_string())?;
            let value = serde_json::from_str::<Value>(&raw).map_err(|error| error.to_string())?;
            if let Some(project_path_filter) = project_path_filter.as_deref() {
                let project_path = value
                    .get("project_path")
                    .and_then(Value::as_str)
                    .and_then(|value| trimmed_option(Some(value)))
                    .map(|value| absolutize_path(&value));
                let working_directory = value
                    .get("working_directory")
                    .and_then(Value::as_str)
                    .and_then(|value| trimmed_option(Some(value)))
                    .map(|value| absolutize_path(&value));
                if project_path.as_deref() != Some(project_path_filter)
                    && working_directory.as_deref() != Some(project_path_filter)
                {
                    continue;
                }
            }
            records.push(value);
        }
    }
    records.sort_by(|left, right| {
        let left_key = run_sort_key(left);
        let right_key = run_sort_key(right);
        right_key.cmp(&left_key)
    });
    Ok(records)
}

fn run_sort_key(value: &Value) -> String {
    value
        .get("started_at")
        .or_else(|| value.get("ended_at"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn resolve_preview_flow_source_dir(
    flow_name: Option<&str>,
    config: &PreviewServiceConfig,
) -> Option<PathBuf> {
    let flow_name = flow_name.unwrap_or_default().trim();
    if flow_name.is_empty() {
        return None;
    }
    let flows_dir = config.flows_dir.as_deref()?;
    resolve_flow_path(flows_dir, flow_name)
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn runtime_error_response(
    error: RuntimeControlError,
    checkpoint_detail: &str,
) -> RuntimeRouteResponse {
    match error {
        RuntimeControlError::UnknownPipeline => {
            RuntimeRouteResponse::json(404, json!({"detail": "Unknown pipeline"}))
        }
        RuntimeControlError::CheckpointUnavailable => {
            RuntimeRouteResponse::json(404, json!({"detail": checkpoint_detail}))
        }
        RuntimeControlError::Validation(error) => {
            RuntimeRouteResponse::json(200, json!({"status": "validation_error", "error": error}))
        }
        RuntimeControlError::Conflict(detail) => {
            RuntimeRouteResponse::json(409, json!({"detail": detail}))
        }
        RuntimeControlError::Storage(error) => {
            RuntimeRouteResponse::json(500, json!({"detail": error.to_string()}))
        }
    }
}

fn child_run_summaries(children: &[RunBundle]) -> Vec<Value> {
    children
        .iter()
        .map(|child| {
            let record = child.record.as_ref();
            json!({
                "run_id": record.map(|record| record.run_id.clone()).unwrap_or_else(|| child.paths.run_id.clone()),
                "record": child.record,
                "checkpoint": child.checkpoint,
                "journal_count": child.journal.len(),
                "event_count": child.raw_events.len(),
            })
        })
        .collect()
}

fn child_journal_groups(children: &[RunBundle]) -> Vec<Value> {
    children
        .iter()
        .map(|child| {
            let record = child.record.as_ref();
            json!({
                "run_id": record.map(|record| record.run_id.clone()).unwrap_or_else(|| child.paths.run_id.clone()),
                "record": child.record,
                "journal": child.journal,
            })
        })
        .collect()
}

fn child_event_groups(children: &[RunBundle]) -> Vec<Value> {
    children
        .iter()
        .map(|child| {
            let record = child.record.as_ref();
            json!({
                "run_id": record.map(|record| record.run_id.clone()).unwrap_or_else(|| child.paths.run_id.clone()),
                "record": child.record,
                "events": child.raw_events,
            })
        })
        .collect()
}
