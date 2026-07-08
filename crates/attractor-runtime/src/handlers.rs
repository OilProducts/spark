use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use attractor_core::{
    attr_text, AttractorContext, ContextMap, DotAttribute, DotGraph, DotValue, FailureKind,
    FlowDefinition, FlowNode, NodeKind, Outcome, OutcomeStatus, RoutingEdge,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_agent_adapter::codergen::CodergenEvent;
use spark_common::debug::{agent_trace_enabled, AGENT_TRACE_FILE_NAME};
use spark_storage::append_jsonl;
use unified_llm_adapter::{
    resolve_effective_llm_model, resolve_effective_llm_profile, resolve_effective_llm_provider,
    resolve_effective_reasoning_effort, LlmResolutionInputs,
};

use crate::artifacts::{
    append_tool_hook_failure, copy_tool_artifact_matches, write_tool_output_log,
    write_tool_text_artifact, ToolHookFailureRecord,
};
use crate::codergen::{codergen_events_for_journal, codergen_outcome, RuntimeCodergen};
use crate::context::{
    clear_runtime_retry_context, seed_builtin_context, set_runtime_fidelity_context,
};
use crate::error::Result;
use crate::events::{
    append_event, interview_completed_event, interview_started_event,
    parallel_branch_completed_event, parallel_branch_started_event, parallel_completed_event,
    parallel_started_event,
};
use crate::executor::{
    panic_payload_message, NodeExecutionRequest, NodeExecutor, RuntimeNodeError,
};
use crate::paths::RunRootPaths;
use crate::routing::{outgoing_routing_edges, select_next_node_with_prior};

pub const HANDLER_START: &str = "start";
pub const HANDLER_EXIT: &str = "exit";
pub const HANDLER_CODERGEN: &str = "codergen";
pub const HANDLER_WAIT_HUMAN: &str = "wait.human";
pub const HANDLER_CONDITIONAL: &str = "conditional";
pub const HANDLER_PARALLEL: &str = "parallel";
pub const HANDLER_FAN_IN: &str = "parallel.fan_in";
pub const HANDLER_TOOL: &str = "tool";
pub const HANDLER_MANAGER_LOOP: &str = "stack.manager_loop";

pub type RuntimeHandlerFn =
    Box<dyn FnMut(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError> + Send>;

pub type RuntimeThreadSafeHandlerFn =
    dyn Fn(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError> + Send + Sync;

pub type FanInRanker = Box<dyn FnMut(FanInRankingRequest) -> Option<String> + Send>;
pub type CodergenBackendFactory =
    Arc<dyn Fn() -> Box<dyn spark_agent_adapter::CodergenBackend> + Send + Sync>;
pub type ChildRunLauncher = Arc<dyn Fn(ChildRunRequest) -> ChildRunResult + Send + Sync>;
pub type ChildStatusResolver = Arc<dyn Fn(&str) -> Option<ChildRunResult> + Send + Sync>;
pub type ChildInterventionRequester =
    Arc<dyn Fn(ChildInterventionRequest) -> ChildInterventionResult + Send + Sync>;

#[derive(Clone)]
enum RegisteredRuntimeHandler {
    Serialized(Arc<Mutex<RuntimeHandlerFn>>),
    ThreadSafe(Arc<RuntimeThreadSafeHandlerFn>),
}

impl RegisteredRuntimeHandler {
    fn execute(&self, runtime: HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError> {
        match self {
            Self::Serialized(handler) => {
                let mut handler = handler
                    .lock()
                    .map_err(|_| RuntimeNodeError::runtime("custom handler lock poisoned"))?;
                handler(runtime)
            }
            Self::ThreadSafe(handler) => handler(runtime),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HandlerRuntime {
    pub node_id: String,
    pub node: FlowNode,
    pub node_attrs: BTreeMap<String, DotAttribute>,
    pub outgoing_edges: Vec<RoutingEdge>,
    pub prompt: String,
    pub context: ContextMap,
    pub flow: FlowDefinition,
    pub(crate) handler_graph: DotGraph,
    pub logs_root: Option<PathBuf>,
    pub artifacts_root: Option<PathBuf>,
    pub run_workdir: PathBuf,
    pub run_id: String,
    pub stage_index: u64,
    pub run_paths: Option<RunRootPaths>,
    pub fallback_model: Option<String>,
    pub fallback_provider: Option<String>,
    pub fallback_profile: Option<String>,
    pub fallback_reasoning_effort: Option<String>,
}

impl HandlerRuntime {
    fn from_request(request: NodeExecutionRequest) -> Self {
        let logs_root = request
            .run_paths
            .as_ref()
            .map(crate::paths::RunRootPaths::logs_dir);
        let artifacts_root = request
            .run_paths
            .as_ref()
            .map(crate::paths::RunRootPaths::artifacts_dir);
        Self {
            node_id: request.node_id,
            node: request.node,
            node_attrs: request.node_attrs,
            outgoing_edges: request.outgoing_edges,
            prompt: request.prompt,
            context: request.context,
            handler_graph: crate::flow_runtime::flow_graph_for_handler_compat(&request.flow),
            flow: request.flow,
            logs_root,
            artifacts_root,
            run_workdir: request.run_workdir,
            run_id: request.run_id,
            stage_index: request.stage_index,
            run_paths: request.run_paths,
            fallback_model: request.fallback_model,
            fallback_provider: request.fallback_provider,
            fallback_profile: request.fallback_profile,
            fallback_reasoning_effort: request.fallback_reasoning_effort,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanOption {
    pub label: String,
    pub value: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanQuestion {
    pub text: String,
    pub stage: String,
    pub options: Vec<HumanOption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanAnswer {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub selected_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_option: Option<HumanOption>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub skipped: bool,
}

impl HumanAnswer {
    pub fn selected(value: impl Into<String>) -> Self {
        Self {
            selected_values: vec![value.into()],
            ..Self::default()
        }
    }

    pub fn skipped() -> Self {
        Self {
            value: "skipped".to_string(),
            skipped: true,
            ..Self::default()
        }
    }
}

pub trait Interviewer {
    fn ask(&mut self, question: HumanQuestion) -> HumanAnswer;
}

#[derive(Debug, Clone, Default)]
pub struct QueueInterviewer {
    answers: VecDeque<HumanAnswer>,
}

impl QueueInterviewer {
    pub fn new(answers: impl IntoIterator<Item = HumanAnswer>) -> Self {
        Self {
            answers: answers.into_iter().collect(),
        }
    }
}

impl Interviewer for QueueInterviewer {
    fn ask(&mut self, _question: HumanQuestion) -> HumanAnswer {
        self.answers
            .pop_front()
            .unwrap_or_else(HumanAnswer::skipped)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AutoApproveInterviewer;

impl Interviewer for AutoApproveInterviewer {
    fn ask(&mut self, question: HumanQuestion) -> HumanAnswer {
        question
            .options
            .first()
            .map(|option| HumanAnswer {
                value: option.value.clone(),
                selected_option: Some(option.clone()),
                ..HumanAnswer::default()
            })
            .unwrap_or_else(HumanAnswer::skipped)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanInRankingRequest {
    pub node_id: String,
    pub prompt: String,
    #[serde(default)]
    pub context: ContextMap,
    #[serde(default)]
    pub candidates: Vec<Value>,
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_profile: String,
    #[serde(default)]
    pub reasoning_effort: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildRunRequest {
    pub child_run_id: String,
    pub child_flow: FlowDefinition,
    pub child_flow_name: String,
    pub child_flow_path: PathBuf,
    pub child_workdir: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input_map: BTreeMap<String, String>,
    #[serde(default)]
    pub parent_context: ContextMap,
    pub parent_run_id: String,
    pub parent_node_id: String,
    pub root_run_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildRunResult {
    pub run_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason_message: Option<String>,
    #[serde(default)]
    pub current_node: String,
    #[serde(default)]
    pub completed_nodes: Vec<String>,
    #[serde(default)]
    pub route_trace: Vec<String>,
    #[serde(default)]
    pub failure_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u64>,
    #[serde(default)]
    pub retry_counts: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_count: Option<usize>,
    #[serde(default)]
    pub checkpoint_timestamp: String,
    #[serde(default)]
    pub latest_event_at: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildInterventionRequest {
    pub child_run_id: String,
    pub message: String,
    pub parent_run_id: String,
    pub parent_node_id: String,
    pub root_run_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "default_manager_loop_source")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_node_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildInterventionResult {
    pub run_id: String,
    pub status: String,
    #[serde(default)]
    pub delivery_mode: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_node_id: Option<String>,
}

fn default_manager_loop_source() -> String {
    "manager_loop".to_string()
}

fn codergen_event_sink(
    runtime: &HandlerRuntime,
) -> Option<spark_agent_adapter::codergen::CodergenEventSink> {
    let paths = runtime.run_paths.clone()?;
    let run_id = runtime.run_id.clone();
    let node_id = runtime.node_id.clone();
    let trace_path = runtime
        .logs_root
        .as_ref()
        .map(|logs_root| logs_root.join(&node_id).join(AGENT_TRACE_FILE_NAME));
    Some(Arc::new(move |event: &CodergenEvent| {
        if let Err(error) =
            crate::transcript::persist_codergen_transcript_event(&paths, &run_id, &node_id, event)
        {
            eprintln!("failed to persist codergen transcript event: {error}");
        }
        if should_write_agent_trace(event) {
            if let Some(trace_path) = trace_path.as_ref() {
                if let Some(parent) = trace_path.parent() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        eprintln!("failed to create agent trace directory: {error}");
                        return;
                    }
                }
                if let Err(error) = append_jsonl(trace_path, event) {
                    eprintln!("failed to append agent trace event: {error}");
                }
            }
        }
    }))
}

fn should_write_agent_trace(event: &CodergenEvent) -> bool {
    agent_trace_enabled()
        && matches!(
            event.event_type.as_str(),
            "rust_agent_session_event" | "rust_agent_raw_log_line"
        )
}

fn adapter_intervention_request_from_runtime(
    request: ChildInterventionRequest,
) -> spark_agent_adapter::CodergenChildInterventionRequest {
    spark_agent_adapter::CodergenChildInterventionRequest {
        child_run_id: request.child_run_id,
        message: request.message,
        parent_run_id: request.parent_run_id,
        parent_node_id: request.parent_node_id,
        root_run_id: request.root_run_id,
        reason: request.reason,
        source: request.source,
        cycle: request.cycle,
        target_node_id: request.target_node_id,
        provider: None,
        model: None,
        llm_profile: None,
        reasoning_effort: None,
    }
}

fn adapter_intervention_result_to_runtime(
    result: spark_agent_adapter::CodergenChildInterventionResult,
) -> ChildInterventionResult {
    ChildInterventionResult {
        run_id: result.run_id,
        status: result.status,
        delivery_mode: result.delivery_mode,
        reason: result.reason,
        message: result.message,
        target_node_id: result.target_node_id,
    }
}

#[derive(Clone)]
pub struct RuntimeHandlerRunner {
    custom_handlers: BTreeMap<String, RegisteredRuntimeHandler>,
    interviewer: Arc<Mutex<Box<dyn Interviewer + Send>>>,
    fan_in_ranker: Option<Arc<Mutex<FanInRanker>>>,
    codergen_backend_factory: Option<CodergenBackendFactory>,
    codergen_intervention_broker: Option<spark_agent_adapter::CodergenSessionInterventionBroker>,
    event_append_lock: Arc<Mutex<()>>,
    child_run_launcher: Option<ChildRunLauncher>,
    child_status_resolver: Option<ChildStatusResolver>,
    child_intervention_requester: Option<ChildInterventionRequester>,
    run_event_observer: Option<crate::store::RunEventObserver>,
    human_gate_blocking: bool,
}

struct BranchCompletion {
    result: std::result::Result<Value, RuntimeNodeError>,
}

impl Default for RuntimeHandlerRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHandlerRunner {
    pub fn new() -> Self {
        Self {
            custom_handlers: BTreeMap::new(),
            interviewer: Arc::new(Mutex::new(Box::<QueueInterviewer>::default())),
            fan_in_ranker: None,
            codergen_backend_factory: None,
            codergen_intervention_broker: None,
            event_append_lock: Arc::new(Mutex::new(())),
            child_run_launcher: None,
            child_status_resolver: None,
            child_intervention_requester: None,
            run_event_observer: None,
            human_gate_blocking: false,
        }
    }

    /// Human gates without a queued answer publish a pending question and
    /// block the (detached) executor thread until the answer route journals
    /// one, honoring persisted cancel/pause requests. Off by default so test
    /// runners keep skip semantics.
    pub fn with_blocking_human_gates(mut self) -> Self {
        self.human_gate_blocking = true;
        self
    }

    pub fn with_run_event_observer(mut self, observer: crate::store::RunEventObserver) -> Self {
        self.run_event_observer = Some(observer);
        self
    }

    pub(crate) fn run_event_observer(&self) -> Option<crate::store::RunEventObserver> {
        self.run_event_observer.clone()
    }

    pub fn with_interviewer(interviewer: impl Interviewer + Send + 'static) -> Self {
        Self {
            interviewer: Arc::new(Mutex::new(Box::new(interviewer))),
            ..Self::new()
        }
    }

    pub fn with_fan_in_ranker(
        mut self,
        ranker: impl FnMut(FanInRankingRequest) -> Option<String> + Send + 'static,
    ) -> Self {
        self.fan_in_ranker = Some(Arc::new(Mutex::new(Box::new(ranker))));
        self
    }

    pub fn with_codergen_backend_factory(
        mut self,
        factory: impl Fn() -> Box<dyn spark_agent_adapter::CodergenBackend> + Send + Sync + 'static,
    ) -> Self {
        self.codergen_backend_factory = Some(Arc::new(factory));
        self.codergen_intervention_broker = None;
        self
    }

    pub fn with_rust_llm_client(mut self, client: unified_llm_adapter::Client) -> Self {
        let broker = spark_agent_adapter::CodergenSessionInterventionBroker::default();
        let factory_broker = broker.clone();
        self.codergen_backend_factory = Some(Arc::new(move || {
            Box::new(
                spark_agent_adapter::RustLlmCodergenBackend::with_intervention_broker(
                    client.clone(),
                    factory_broker.clone(),
                ),
            )
        }));
        self.codergen_intervention_broker = Some(broker);
        self
    }

    pub fn set_codergen_backend_factory(
        &mut self,
        factory: impl Fn() -> Box<dyn spark_agent_adapter::CodergenBackend> + Send + Sync + 'static,
    ) {
        self.codergen_backend_factory = Some(Arc::new(factory));
        self.codergen_intervention_broker = None;
    }

    pub fn set_rust_llm_client(&mut self, client: unified_llm_adapter::Client) {
        let broker = spark_agent_adapter::CodergenSessionInterventionBroker::default();
        let factory_broker = broker.clone();
        self.codergen_backend_factory = Some(Arc::new(move || {
            Box::new(
                spark_agent_adapter::RustLlmCodergenBackend::with_intervention_broker(
                    client.clone(),
                    factory_broker.clone(),
                ),
            )
        }));
        self.codergen_intervention_broker = Some(broker);
    }

    pub fn with_child_run_launcher(
        mut self,
        launcher: impl Fn(ChildRunRequest) -> ChildRunResult + Send + Sync + 'static,
    ) -> Self {
        self.child_run_launcher = Some(Arc::new(launcher));
        self
    }

    pub fn set_child_run_launcher(
        &mut self,
        launcher: impl Fn(ChildRunRequest) -> ChildRunResult + Send + Sync + 'static,
    ) {
        self.child_run_launcher = Some(Arc::new(launcher));
    }

    pub fn with_child_status_resolver(
        mut self,
        resolver: impl Fn(&str) -> Option<ChildRunResult> + Send + Sync + 'static,
    ) -> Self {
        self.child_status_resolver = Some(Arc::new(resolver));
        self
    }

    pub fn set_child_status_resolver(
        &mut self,
        resolver: impl Fn(&str) -> Option<ChildRunResult> + Send + Sync + 'static,
    ) {
        self.child_status_resolver = Some(Arc::new(resolver));
    }

    pub fn with_child_intervention_requester(
        mut self,
        requester: impl Fn(ChildInterventionRequest) -> ChildInterventionResult + Send + Sync + 'static,
    ) -> Self {
        self.child_intervention_requester = Some(Arc::new(requester));
        self
    }

    pub fn set_child_intervention_requester(
        &mut self,
        requester: impl Fn(ChildInterventionRequest) -> ChildInterventionResult + Send + Sync + 'static,
    ) {
        self.child_intervention_requester = Some(Arc::new(requester));
    }

    pub(crate) fn child_run_launcher(&self) -> Option<ChildRunLauncher> {
        self.child_run_launcher.clone()
    }

    pub(crate) fn child_status_resolver(&self) -> Option<ChildStatusResolver> {
        self.child_status_resolver.clone()
    }

    pub(crate) fn child_intervention_requester(&self) -> Option<ChildInterventionRequester> {
        if let Some(requester) = self.child_intervention_requester.clone() {
            return Some(requester);
        }
        let broker = self.codergen_intervention_broker.clone()?;
        Some(Arc::new(move |request| {
            adapter_intervention_result_to_runtime(
                broker
                    .request_child_intervention(adapter_intervention_request_from_runtime(request)),
            )
        }))
    }

    pub fn request_child_intervention(
        &self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult {
        if let Some(requester) = self.child_intervention_requester() {
            return requester(request.clone());
        }
        ChildInterventionResult {
            run_id: request.child_run_id,
            status: "rejected".to_string(),
            delivery_mode: "unsupported".to_string(),
            reason: "backend_steering_unsupported".to_string(),
            message: "child intervention requester is unavailable".to_string(),
            target_node_id: request.target_node_id,
        }
    }

    pub fn register_handler_fn(
        &mut self,
        handler_type: impl Into<String>,
        handler: impl FnMut(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError>
            + Send
            + 'static,
    ) {
        self.custom_handlers.insert(
            handler_type.into(),
            RegisteredRuntimeHandler::Serialized(Arc::new(Mutex::new(Box::new(handler)))),
        );
    }

    pub fn register_thread_safe_handler_fn(
        &mut self,
        handler_type: impl Into<String>,
        handler: impl Fn(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError>
            + Send
            + Sync
            + 'static,
    ) {
        self.custom_handlers.insert(
            handler_type.into(),
            RegisteredRuntimeHandler::ThreadSafe(Arc::new(handler)),
        );
    }

    pub fn register_static_handler(&mut self, handler_type: impl Into<String>, outcome: Outcome) {
        self.register_thread_safe_handler_fn(handler_type, move |_runtime| Ok(outcome.clone()));
    }

    pub fn resolve_handler_type(&self, node: &FlowNode) -> String {
        crate::flow_runtime::handler_type_for_node(node).to_string()
    }

    fn execute_request(
        &self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let runtime = HandlerRuntime::from_request(request);
        self.execute_runtime(runtime)
    }

    fn execute_runtime(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let handler_type = self.resolve_handler_type(&runtime.node);
        if let Some(handler) = self.custom_handlers.get(&handler_type) {
            return handler.execute(runtime);
        }

        match handler_type.as_str() {
            HANDLER_START | HANDLER_EXIT | HANDLER_CONDITIONAL => {
                Ok(Outcome::new(OutcomeStatus::Success))
            }
            HANDLER_TOOL => self.execute_tool(runtime),
            HANDLER_WAIT_HUMAN => self.execute_human(runtime),
            HANDLER_PARALLEL => self.execute_parallel(runtime),
            HANDLER_FAN_IN => self.execute_fan_in(runtime),
            HANDLER_CODERGEN => self.execute_codergen(runtime),
            HANDLER_MANAGER_LOOP => crate::manager_loop::execute_manager_loop(self, runtime),
            _ => self.execute_codergen(runtime),
        }
    }

    pub(crate) fn emit(
        &self,
        runtime: &HandlerRuntime,
        event: attractor_core::RawRuntimeEvent,
    ) -> Result<()> {
        if let Some(paths) = &runtime.run_paths {
            let _guard = self.event_append_lock.lock().map_err(|_| {
                crate::error::RuntimeStorageError::io(
                    "lock runtime event append",
                    paths.events_jsonl(),
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "runtime event append lock poisoned",
                    ),
                )
            })?;
            append_event(paths, event)?;
            if let Some(observer) = &self.run_event_observer {
                observer(&paths.run_id);
            }
        }
        Ok(())
    }

    pub(crate) fn emit_transcript_event(
        &self,
        runtime: &HandlerRuntime,
        event: attractor_core::RawRuntimeEvent,
    ) -> Result<()> {
        if let Some(paths) = &runtime.run_paths {
            let _guard = self.event_append_lock.lock().map_err(|_| {
                crate::error::RuntimeStorageError::io(
                    "lock runtime event append",
                    paths.events_jsonl(),
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "runtime event append lock poisoned",
                    ),
                )
            })?;
            let event = append_event(paths, event)?;
            crate::transcript::persist_transcript_runtime_event(paths, &event)?;
        }
        Ok(())
    }

    fn execute_codergen(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let root_run_id = runtime
            .context
            .get("internal.root_run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(runtime.run_id.as_str())
            .to_string();
        let mut codergen = if let Some(factory) = self.codergen_backend_factory.as_ref() {
            RuntimeCodergen::with_boxed_backend(
                runtime.handler_graph.clone(),
                runtime.logs_root.clone(),
                factory(),
            )
        } else {
            RuntimeCodergen::simulation(runtime.handler_graph.clone(), runtime.logs_root.clone())
        }
        .with_llm_fallbacks(
            runtime.fallback_model.clone(),
            runtime.fallback_provider.clone(),
            runtime.fallback_profile.clone(),
            runtime.fallback_reasoning_effort.clone(),
        )
        .with_runtime_context(
            Some(runtime.run_workdir.clone()),
            BTreeMap::from([
                (
                    "spark.runtime.run_id".to_string(),
                    json!(runtime.run_id.clone()),
                ),
                ("spark.runtime.root_run_id".to_string(), json!(root_run_id)),
                (
                    "spark.runtime.run_workdir".to_string(),
                    json!(runtime.run_workdir.to_string_lossy().to_string()),
                ),
            ]),
        )
        .with_event_sink(self.codergen_event_sink(&runtime));
        // Journal codergen events as they are produced so transcripts stream
        // while the node executes; the event-log prefix contract means every
        // event is sunk exactly once, so no post-hoc pass is needed. The
        // transcript sink above stays in place until the transcript.json
        // write path is retired in favor of the journal projection.
        let live_sink_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let live_sink = runtime.run_paths.as_ref().map(|paths| {
            let paths = paths.clone();
            let run_id = runtime.run_id.clone();
            let node_id = runtime.node_id.clone();
            let append_lock = Arc::clone(&self.event_append_lock);
            let run_event_observer = self.run_event_observer.clone();
            let sink_error = Arc::clone(&live_sink_error);
            Arc::new(move |event: spark_agent_adapter::CodergenEvent| {
                let raw_event = crate::events::codergen_adapter_event(
                    &run_id,
                    &node_id,
                    &event.event_type,
                    serde_json::to_value(&event.payload).unwrap_or_else(|_| json!({})),
                );
                let append_result = match append_lock.lock() {
                    Ok(_guard) => crate::events::append_event(&paths, raw_event).map(|_| ()),
                    Err(_) => Err(crate::error::RuntimeStorageError::io(
                        "lock runtime event append",
                        paths.events_jsonl(),
                        std::io::Error::other("runtime event append lock poisoned"),
                    )),
                };
                match append_result {
                    Ok(()) => {
                        if let Some(observer) = &run_event_observer {
                            observer(&run_id);
                        }
                    }
                    Err(error) => {
                        let mut slot = sink_error
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        if slot.is_none() {
                            *slot = Some(error.to_string());
                        }
                    }
                }
            }) as spark_agent_adapter::CodergenStreamSink
        });
        let journaling_live = live_sink.is_some();
        let execution = codergen
            .execute_with_event_sink(&runtime.node_id, runtime.context.clone(), live_sink)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        if let Some(error) = live_sink_error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            return Err(RuntimeNodeError::runtime(error));
        }
        if let Some(paths) = runtime.run_paths.as_ref() {
            crate::transcript::persist_codergen_final_response_text(
                paths,
                &runtime.node_id,
                &execution.response_text,
            )
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        }
        if runtime.run_paths.is_some() && !journaling_live {
            for event in codergen_events_for_journal(&runtime.run_id, &runtime.node_id, &execution)
            {
                self.emit(&runtime, event)
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            }
        }
        Ok(codergen_outcome(execution))
    }

    fn codergen_event_sink(
        &self,
        runtime: &HandlerRuntime,
    ) -> Option<spark_agent_adapter::codergen::CodergenEventSink> {
        codergen_event_sink(runtime)
    }

    fn execute_tool(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let Some(command) = attr_text(&runtime.node_attrs, "tool.command") else {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "No tool.command specified".to_string(),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        };
        let cwd = tool_cwd(&runtime);
        let hook_metadata = json!({
            "hook_phase": "",
            "node_id": runtime.node_id,
            "tool_command": command,
        });

        if let Some(pre_hook) = resolve_hook_command(&runtime, "tool.hooks.pre") {
            let pre_hook_result = run_hook(&pre_hook, "pre", &runtime, &command, &cwd);
            record_hook_failure(&runtime, &pre_hook, "pre", &pre_hook_result)?;
            if pre_hook_result.exit_code != 0 {
                write_tool_output_if_available(&runtime, "")?;
                return Ok(Outcome {
                    status: OutcomeStatus::Fail,
                    failure_reason: pre_hook_failure_reason(&pre_hook, &pre_hook_result),
                    context_updates: tool_context_updates("", -1),
                    ..Outcome::new(OutcomeStatus::Fail)
                });
            }
        }

        let timeout = timeout_seconds(&runtime.node_attrs).map(Duration::from_secs_f64);
        let command_output = match run_shell_command(&command, &cwd, timeout, BTreeMap::new(), None)
        {
            Ok(output) => output,
            Err(error) => ShellOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: error.to_string(),
                timed_out: false,
            },
        };
        write_tool_output_if_available(&runtime, &command_output.stdout)?;

        let mut outcome = if command_output.timed_out {
            Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: timeout_failure_reason(&command, timeout),
                context_updates: tool_context_updates(command_output.stdout.trim(), -1),
                ..Outcome::new(OutcomeStatus::Fail)
            }
        } else if command_output.exit_code == 0 {
            let notes = command_output.stdout.trim().to_string();
            Outcome {
                status: OutcomeStatus::Success,
                notes: notes.clone(),
                context_updates: tool_context_updates(&notes, 0),
                ..Outcome::new(OutcomeStatus::Success)
            }
        } else {
            let reason = command_output.stderr.trim();
            Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: if reason.is_empty() {
                    format!("tool command failed with code {}", command_output.exit_code)
                } else {
                    reason.to_string()
                },
                context_updates: tool_context_updates(
                    command_output.stdout.trim(),
                    command_output.exit_code,
                ),
                ..Outcome::new(OutcomeStatus::Fail)
            }
        };

        if let Some(post_hook) = resolve_hook_command(&runtime, "tool.hooks.post") {
            let post_hook_result = run_hook(&post_hook, "post", &runtime, &command, &cwd);
            record_hook_failure(&runtime, &post_hook, "post", &post_hook_result)?;
        }

        if let Some(reason) = capture_declared_artifacts(
            &runtime,
            &cwd,
            &command_output.stdout,
            &command_output.stderr,
        )? {
            outcome.status = OutcomeStatus::Fail;
            outcome.failure_reason = reason;
            outcome.retryable = Some(false);
            outcome.failure_kind = Some(FailureKind::Runtime);
        }

        let _ = hook_metadata;
        Ok(outcome)
    }

    fn execute_human(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        if runtime.outgoing_edges.is_empty() {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "No outgoing edges for human gate".to_string(),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }

        let choices = runtime
            .outgoing_edges
            .iter()
            .map(|edge| {
                let label = if edge.label.trim().is_empty() {
                    edge.target.as_str().to_string()
                } else {
                    edge.label.clone()
                };
                Choice {
                    key: parse_accelerator_key(&label),
                    label,
                    target: edge.target.as_str().to_string(),
                }
            })
            .collect::<Vec<_>>();
        let question_text = if runtime.prompt.trim().is_empty() {
            "Choose next route".to_string()
        } else {
            runtime.prompt.clone()
        };
        let question = HumanQuestion {
            text: question_text.clone(),
            stage: runtime.node_id.clone(),
            options: choices
                .iter()
                .map(|choice| HumanOption {
                    label: choice.label.clone(),
                    value: choice.label.clone(),
                    key: choice.key.clone(),
                })
                .collect(),
        };

        self.emit(
            &runtime,
            interview_started_event(&runtime.run_id, &runtime.node_id, &question_text),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        let mut answer = self
            .interviewer
            .lock()
            .map_err(|_| RuntimeNodeError::runtime("interviewer lock poisoned"))?
            .ask(question);
        if (answer.skipped || answer.value == "skipped")
            && self.human_gate_blocking
            && runtime.run_paths.is_some()
        {
            match self.wait_for_human_gate_answer(&runtime, &question_text, &choices)? {
                HumanGateWaitResult::Answered(waited_answer) => {
                    answer = waited_answer;
                }
                HumanGateWaitResult::Interrupted(outcome) => {
                    return Ok(outcome);
                }
            }
        }
        if answer.skipped || answer.value == "skipped" {
            self.emit_transcript_event(
                &runtime,
                interview_completed_event(
                    &runtime.run_id,
                    &runtime.node_id,
                    &question_text,
                    answer.value,
                    "skipped",
                ),
            )
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            return Ok(human_skipped_outcome());
        }

        let selected = select_choice(&answer, &choices);
        let emitted_answer = selected
            .as_ref()
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| answer.value.clone());
        self.emit_transcript_event(
            &runtime,
            interview_completed_event(
                &runtime.run_id,
                &runtime.node_id,
                &question_text,
                emitted_answer,
                if selected.is_some() {
                    "accepted"
                } else {
                    "skipped"
                },
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let Some(selected) = selected else {
            return Ok(human_skipped_outcome());
        };
        Ok(Outcome {
            status: OutcomeStatus::Success,
            preferred_label: selected.label.clone(),
            suggested_next_ids: vec![selected.target.clone()],
            context_updates: ContextMap::from([
                ("human.gate.selected".to_string(), json!(selected.key)),
                ("human.gate.label".to_string(), json!(selected.label)),
            ]),
            notes: "human selection applied".to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        })
    }

    /// Publishes the pending question, marks the run `waiting`, and polls the
    /// journal until the answer route records an InterviewCompleted for this
    /// question — or a persisted cancel/pause request interrupts the wait.
    fn wait_for_human_gate_answer(
        &self,
        runtime: &HandlerRuntime,
        question_text: &str,
        choices: &[Choice],
    ) -> std::result::Result<HumanGateWaitResult, RuntimeNodeError> {
        let paths = runtime
            .run_paths
            .clone()
            .ok_or_else(|| RuntimeNodeError::runtime("human gate wait requires run paths"))?;
        let question_id = format!("{}-{}", runtime.node_id, runtime.stage_index);
        let flow_name = crate::records::read_run_record(&paths)
            .ok()
            .flatten()
            .map(|record| record.flow_name)
            .unwrap_or_default();
        let options = choices
            .iter()
            .map(|choice| {
                json!({
                    "label": choice.label,
                    "value": choice.label,
                    "key": choice.key,
                })
            })
            .collect::<Vec<_>>();
        self.emit(
            runtime,
            crate::events::human_gate_pending_event(
                &runtime.run_id,
                &question_id,
                &runtime.node_id,
                &flow_name,
                question_text,
                options,
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        self.write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Waiting)?;

        loop {
            if let Some(answer_value) = journaled_gate_answer(&paths, &question_id) {
                self.write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Running)?;
                return Ok(HumanGateWaitResult::Answered(HumanAnswer {
                    value: answer_value.clone(),
                    selected_values: vec![answer_value.clone()],
                    selected_option: None,
                    text: answer_value,
                    skipped: false,
                }));
            }
            let status = crate::records::read_run_record(&paths)
                .ok()
                .flatten()
                .map(|record| crate::records::normalize_run_status(&record.status))
                .unwrap_or_default();
            match status.as_str() {
                "cancel_requested" | "canceled" => {
                    // Fail non-retryably; the executor's post-stage control
                    // poll then finalizes the run as canceled.
                    return Ok(HumanGateWaitResult::Interrupted(Outcome {
                        status: OutcomeStatus::Fail,
                        failure_reason: "Run cancel requested while waiting for human input"
                            .to_string(),
                        retryable: Some(false),
                        failure_kind: Some(FailureKind::Runtime),
                        ..Outcome::new(OutcomeStatus::Fail)
                    }));
                }
                "pause_requested" | "paused" => {
                    return Ok(HumanGateWaitResult::Interrupted(Outcome {
                        status: OutcomeStatus::Fail,
                        failure_reason: "Run paused while waiting for human input".to_string(),
                        retryable: Some(false),
                        failure_kind: Some(FailureKind::Runtime),
                        ..Outcome::new(OutcomeStatus::Fail)
                    }));
                }
                _ => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(
                HUMAN_GATE_POLL_INTERVAL_MS,
            ));
        }
    }

    fn write_gate_run_status(
        &self,
        paths: &RunRootPaths,
        run_id: &str,
        status: GateRunStatus,
    ) -> std::result::Result<(), RuntimeNodeError> {
        let Some(mut record) = crate::records::read_run_record(paths)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?
        else {
            return Ok(());
        };
        // Never clobber an in-flight control request with a gate transition.
        let current = crate::records::normalize_run_status(&record.status);
        if matches!(
            current.as_str(),
            "cancel_requested" | "canceled" | "pause_requested" | "paused"
        ) {
            return Ok(());
        }
        match status {
            GateRunStatus::Waiting => crate::records::mark_record_waiting(&mut record),
            GateRunStatus::Running => crate::records::mark_record_running_after_wait(&mut record),
        }
        crate::records::write_run_record(paths, &record)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        if let Some(observer) = &self.run_event_observer {
            observer(run_id);
        }
        Ok(())
    }

    fn execute_parallel(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        if runtime.outgoing_edges.is_empty() {
            return Ok(fail_outcome("parallel node has no outgoing edges"));
        }

        let parallel_config = match runtime.node.config.as_ref() {
            Some(attractor_core::NodeConfig::Parallel {
                join_policy,
                max_parallel,
                join_k,
                join_quorum,
            }) => (join_policy.clone(), *max_parallel, *join_k, *join_quorum),
            _ => (None, None, None, None),
        };
        let join_policy = parallel_config.0.unwrap_or_else(|| "wait_all".to_string());
        if !matches!(
            join_policy.as_str(),
            "wait_all" | "k_of_n" | "first_success" | "quorum"
        ) {
            return Ok(fail_outcome(format!(
                "unsupported join_policy: {join_policy}"
            )));
        }
        let error_policy = runtime
            .node
            .runtime
            .as_ref()
            .and_then(|config| config.error_policy.clone())
            .unwrap_or_else(|| "continue".to_string());
        if !matches!(error_policy.as_str(), "fail_fast" | "continue" | "ignore") {
            return Ok(fail_outcome(format!(
                "unsupported error_policy: {error_policy}"
            )));
        }
        let max_parallel = parallel_config.1.unwrap_or(4) as i64;
        if max_parallel < 1 {
            return Ok(fail_outcome("max_parallel must be >= 1"));
        }
        if let Some(error) = validate_join_thresholds_for_config(
            &join_policy,
            parallel_config.2,
            parallel_config.3,
            runtime.outgoing_edges.len(),
        ) {
            return Ok(fail_outcome(error));
        }

        self.emit(
            &runtime,
            parallel_started_event(
                &runtime.run_id,
                &runtime.node_id,
                runtime.outgoing_edges.len(),
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let branches = runtime
            .outgoing_edges
            .iter()
            .enumerate()
            .map(|(index, edge)| (index, edge.target.as_str().to_string()))
            .collect::<Vec<_>>();
        let results = self.execute_parallel_branches(
            runtime.clone(),
            branches,
            max_parallel as usize,
            &join_policy,
            &error_policy,
        )?;

        let event_success_count = results
            .iter()
            .filter(|result| branch_succeeded(result))
            .count();
        let event_fail_count = results
            .iter()
            .filter(|result| branch_failed(result))
            .count();
        self.emit(
            &runtime,
            parallel_completed_event(
                &runtime.run_id,
                &runtime.node_id,
                event_success_count,
                event_fail_count,
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let results_for_policy = if error_policy == "ignore" {
            results
                .iter()
                .filter(|result| branch_succeeded(result))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            results.clone()
        };
        let success_count = results_for_policy
            .iter()
            .filter(|result| branch_succeeded(result))
            .count();
        let fail_count = results_for_policy
            .iter()
            .filter(|result| branch_failed(result))
            .count();
        let status = match join_policy.as_str() {
            "wait_all" => {
                if fail_count == 0 {
                    OutcomeStatus::Success
                } else {
                    OutcomeStatus::PartialSuccess
                }
            }
            "first_success" => {
                if success_count > 0 {
                    OutcomeStatus::Success
                } else {
                    OutcomeStatus::Fail
                }
            }
            "k_of_n" => {
                let required = parallel_config
                    .2
                    .map(|value| value as i64)
                    .unwrap_or(results_for_policy.len() as i64);
                if success_count as i64 >= required {
                    OutcomeStatus::Success
                } else {
                    OutcomeStatus::Fail
                }
            }
            "quorum" => {
                let quorum = parallel_config.3.unwrap_or(0.5);
                let required = ((results_for_policy.len() as f64) * quorum).ceil().max(1.0);
                if (success_count as f64) >= required {
                    OutcomeStatus::Success
                } else {
                    OutcomeStatus::Fail
                }
            }
            _ => OutcomeStatus::Fail,
        };

        Ok(Outcome {
            status,
            context_updates: ContextMap::from([(
                "parallel.results".to_string(),
                json!(results_for_policy),
            )]),
            notes: "parallel fan-out completed".to_string(),
            ..Outcome::new(status)
        })
    }

    fn execute_parallel_branches(
        &self,
        runtime: HandlerRuntime,
        branches: Vec<(usize, String)>,
        max_parallel: usize,
        join_policy: &str,
        error_policy: &str,
    ) -> std::result::Result<Vec<Value>, RuntimeNodeError> {
        let max_parallel = max_parallel.min(branches.len()).max(1);
        let (tx, rx) = mpsc::channel::<BranchCompletion>();

        thread::scope(
            |scope| -> std::result::Result<Vec<Value>, RuntimeNodeError> {
                let mut next_branch = 0_usize;
                let mut in_flight = 0_usize;
                let mut accepting_results = true;
                let mut results = Vec::<Value>::new();

                macro_rules! launch_branch {
                    ($index:expr, $branch:expr) => {{
                        let branch_runtime = runtime.clone();
                        let branch_tx = tx.clone();
                        let branch_id = $branch;
                        let branch_index = $index;
                        scope.spawn(move || {
                            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                || -> std::result::Result<Value, RuntimeNodeError> {
                                    self.emit(
                                        &branch_runtime,
                                        parallel_branch_started_event(
                                            &branch_runtime.run_id,
                                            &branch_runtime.node_id,
                                            &branch_id,
                                            branch_index,
                                        ),
                                    )
                                    .map_err(|error| {
                                        RuntimeNodeError::runtime(error.to_string())
                                    })?;
                                    let payload =
                                        self.execute_branch_from(&branch_runtime, &branch_id)?;
                                    let success = branch_succeeded(&payload);
                                    self.emit(
                                        &branch_runtime,
                                        parallel_branch_completed_event(
                                            &branch_runtime.run_id,
                                            &branch_runtime.node_id,
                                            &branch_id,
                                            branch_index,
                                            success,
                                        ),
                                    )
                                    .map_err(|error| {
                                        RuntimeNodeError::runtime(error.to_string())
                                    })?;
                                    Ok(payload)
                                },
                            ))
                            .unwrap_or_else(|payload| {
                                Err(RuntimeNodeError::runtime(panic_payload_message(payload)))
                            });
                            let _ = branch_tx.send(BranchCompletion { result });
                        });
                    }};
                }

                while accepting_results && in_flight < max_parallel && next_branch < branches.len()
                {
                    let (index, branch) = branches[next_branch].clone();
                    launch_branch!(index, branch);
                    next_branch += 1;
                    in_flight += 1;
                }

                while in_flight > 0 {
                    let completion = rx.recv().map_err(|_| {
                        RuntimeNodeError::runtime("parallel branch worker disconnected")
                    })?;
                    in_flight -= 1;

                    if accepting_results {
                        let payload = completion.result?;
                        let success = branch_succeeded(&payload);
                        let failed = branch_failed(&payload);
                        results.push(payload);

                        if error_policy == "fail_fast" && failed {
                            accepting_results = false;
                        }
                        if join_policy == "first_success" && success {
                            accepting_results = false;
                        }
                    }

                    while accepting_results
                        && in_flight < max_parallel
                        && next_branch < branches.len()
                    {
                        let (index, branch) = branches[next_branch].clone();
                        launch_branch!(index, branch);
                        next_branch += 1;
                        in_flight += 1;
                    }
                }

                Ok(results)
            },
        )
    }

    fn execute_branch_from(
        &self,
        runtime: &HandlerRuntime,
        start_node: &str,
    ) -> std::result::Result<Value, RuntimeNodeError> {
        let fan_in_nodes = fan_in_nodes(&runtime.flow);
        let mut values = crate::flow_runtime::flow_context_seed(&runtime.flow);
        for (key, value) in &runtime.context {
            values.insert(key.clone(), value.clone());
        }
        let mut context = AttractorContext::from_map(values)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        seed_builtin_context(&mut context, start_node)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        clear_runtime_retry_context(&mut context)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let mut current_node = start_node.to_string();
        let mut completed_nodes = Vec::<String>::new();
        let mut node_outcomes = BTreeMap::<String, Outcome>::new();
        let max_steps = runtime
            .flow
            .nodes
            .len()
            .saturating_add(runtime.flow.edges.len())
            .max(1);
        for _ in 0..=max_steps {
            if fan_in_nodes.contains(&current_node) {
                return Ok(branch_payload(
                    start_node,
                    "completed",
                    Some("success"),
                    &current_node,
                    &completed_nodes,
                    &context,
                    &node_outcomes,
                    "",
                ));
            }
            let Some(node) = runtime.flow.nodes.get(&current_node).cloned() else {
                return Ok(branch_payload(
                    start_node,
                    "failed",
                    None,
                    &current_node,
                    &completed_nodes,
                    &context,
                    &node_outcomes,
                    &format!("Unknown runtime node: {current_node}"),
                ));
            };
            context
                .set("current_node", json!(current_node.clone()))
                .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            set_runtime_fidelity_context(&runtime.flow, &current_node, None, &mut context, None)
                .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            let prior_status = context
                .get("outcome")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<OutcomeStatus>().ok());
            let prior_preferred_label = context
                .get("preferred_label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let outgoing_edges = outgoing_routing_edges(&runtime.flow, &current_node)
                .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            let node_attrs = crate::flow_runtime::node_attrs_for_handler(&current_node, &node);
            let prompt = crate::flow_runtime::node_prompt(&node);
            let request = NodeExecutionRequest {
                node_id: current_node.clone(),
                stage_index: completed_nodes.len() as u64,
                context: context.snapshot(),
                prompt,
                node: node.clone(),
                node_attrs: node_attrs.clone(),
                flow: runtime.flow.clone(),
                outgoing_edges,
                run_paths: runtime.run_paths.clone(),
                run_workdir: runtime.run_workdir.clone(),
                run_id: runtime.run_id.clone(),
                fallback_model: runtime.fallback_model.clone(),
                fallback_provider: runtime.fallback_provider.clone(),
                fallback_profile: runtime.fallback_profile.clone(),
                fallback_reasoning_effort: runtime.fallback_reasoning_effort.clone(),
            };
            let raw_outcome = match self.execute_request(request) {
                Ok(outcome) => outcome,
                Err(error) => handler_error_outcome(error),
            };
            let outcome = crate::context::apply_outcome_context_updates(
                &current_node,
                &node_attrs,
                &mut context,
                &raw_outcome,
            )
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            node_outcomes.insert(current_node.clone(), outcome.clone());
            completed_nodes.push(current_node.clone());

            let selection = select_next_node_with_prior(
                &runtime.flow,
                &current_node,
                &outcome,
                &context,
                prior_status,
                &prior_preferred_label,
            )
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            let Some(next_node) = selection.selected_node else {
                if outcome.status == OutcomeStatus::Fail {
                    return Ok(branch_payload(
                        start_node,
                        "failed",
                        None,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &node_outcomes,
                        &stage_failure_reason(&outcome),
                    ));
                }
                return Ok(branch_payload(
                    start_node,
                    "completed",
                    Some("success"),
                    &current_node,
                    &completed_nodes,
                    &context,
                    &node_outcomes,
                    "",
                ));
            };
            current_node = next_node;
        }

        Ok(branch_payload(
            start_node,
            "failed",
            None,
            &current_node,
            &completed_nodes,
            &context,
            &node_outcomes,
            "parallel branch exceeded step limit",
        ))
    }

    fn execute_fan_in(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let raw_results = runtime
            .context
            .get("parallel.results")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let results = normalize_parallel_results(&raw_results);
        if results.is_empty() {
            return Ok(fail_outcome("No parallel results to evaluate"));
        }
        let mut candidates = results
            .into_iter()
            .filter(|result| {
                result_status_rank(&result_status(result)) < result_status_rank("fail")
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(fail_outcome("All parallel branches failed"));
        }

        let mut selected: Option<Value> = None;
        if !runtime.prompt.trim().is_empty() {
            if let Some(ranker) = self.fan_in_ranker.as_ref() {
                let resolution = fan_in_llm_resolution_inputs(&runtime);
                let request = FanInRankingRequest {
                    node_id: runtime.node_id.clone(),
                    prompt: runtime.prompt.clone(),
                    context: runtime.context.clone(),
                    candidates: candidates.clone(),
                    provider: resolve_effective_llm_provider(&resolution, &runtime.context),
                    model: resolve_effective_llm_model(&resolution, &runtime.context),
                    llm_profile: resolve_effective_llm_profile(&resolution, &runtime.context)
                        .unwrap_or_default(),
                    reasoning_effort: resolve_effective_reasoning_effort(
                        &resolution,
                        &runtime.context,
                    )
                    .unwrap_or_default(),
                };
                let best_id = {
                    let mut ranker = ranker
                        .lock()
                        .map_err(|_| RuntimeNodeError::runtime("fan-in ranker lock poisoned"))?;
                    ranker(request).and_then(|value| extract_best_id(&value))
                };
                if let Some(best_id) = best_id {
                    selected = candidates
                        .iter()
                        .find(|candidate| object_string(candidate, "id") == best_id)
                        .cloned();
                }
            }
        }

        let best = selected.unwrap_or_else(|| {
            candidates.sort_by(compare_fan_in_candidates);
            candidates.remove(0)
        });
        let best_id = object_string(&best, "id");
        let best_outcome = result_status(&best);
        Ok(Outcome {
            status: OutcomeStatus::Success,
            context_updates: ContextMap::from([
                (
                    "parallel.fan_in.best_id".to_string(),
                    json!(best_id.clone()),
                ),
                (
                    "parallel.fan_in.best_outcome".to_string(),
                    json!(best_outcome),
                ),
            ]),
            notes: format!("Selected best candidate: {best_id}"),
            ..Outcome::new(OutcomeStatus::Success)
        })
    }
}

impl NodeExecutor for RuntimeHandlerRunner {
    fn execute(
        &mut self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.execute_request(request)
        })) {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(error)) => Ok(handler_error_outcome(error)),
            Err(payload) => Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: panic_payload_message(payload),
                retryable: Some(false),
                failure_kind: Some(FailureKind::Runtime),
                ..Outcome::new(OutcomeStatus::Fail)
            }),
        }
    }
}

fn fan_in_llm_resolution_inputs(runtime: &HandlerRuntime) -> LlmResolutionInputs {
    let node_attrs = &runtime.node_attrs;
    let reasoning_attr = node_attrs.get("reasoning_effort");
    LlmResolutionInputs {
        node_model: attr_text(node_attrs, "llm_model"),
        node_provider: attr_text(node_attrs, "llm_provider"),
        node_profile: attr_text(node_attrs, "llm_profile"),
        node_reasoning_effort: attr_text(node_attrs, "reasoning_effort"),
        node_reasoning_is_default_placeholder: reasoning_attr.is_some_and(|attr| attr.line == 0),
        fallback_model: runtime.fallback_model.clone(),
        fallback_provider: runtime.fallback_provider.clone(),
        fallback_profile: runtime.fallback_profile.clone(),
        fallback_reasoning_effort: runtime.fallback_reasoning_effort.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    key: String,
    label: String,
    target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn handler_error_outcome(error: RuntimeNodeError) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: error.message,
        retryable: error.retryable,
        failure_kind: Some(error.failure_kind.unwrap_or(FailureKind::Runtime)),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

fn fail_outcome(reason: impl Into<String>) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: reason.into(),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

fn tool_context_updates(output: &str, exit_code: i32) -> ContextMap {
    ContextMap::from([
        ("context.tool.output".to_string(), json!(output)),
        ("context.tool.exit_code".to_string(), json!(exit_code)),
    ])
}

fn tool_cwd(runtime: &HandlerRuntime) -> PathBuf {
    runtime
        .context
        .get("internal.run_workdir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime.run_workdir.clone())
}

fn resolve_hook_command(runtime: &HandlerRuntime, key: &str) -> Option<String> {
    attr_text(&runtime.node_attrs, key)
        .or_else(|| flow_metadata_text(&runtime.flow, key))
        .filter(|value| !value.trim().is_empty())
}

fn flow_metadata_text(flow: &FlowDefinition, key: &str) -> Option<String> {
    flow.metadata
        .get(key)
        .or_else(|| flow.extensions.get(key))
        .and_then(value_text)
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn run_hook(
    command: &str,
    hook_phase: &str,
    runtime: &HandlerRuntime,
    tool_command: &str,
    cwd: &Path,
) -> ShellOutput {
    let payload = json!({
        "hook_phase": hook_phase,
        "node_id": runtime.node_id,
        "tool_command": tool_command,
    });
    let mut env = BTreeMap::new();
    env.insert(
        "ATTRACTOR_TOOL_HOOK_PHASE".to_string(),
        hook_phase.to_string(),
    );
    env.insert(
        "ATTRACTOR_TOOL_NODE_ID".to_string(),
        runtime.node_id.clone(),
    );
    env.insert(
        "ATTRACTOR_TOOL_COMMAND".to_string(),
        tool_command.to_string(),
    );
    run_shell_command(command, cwd, None, env, Some(payload.to_string())).unwrap_or_else(|error| {
        ShellOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: error.to_string(),
            timed_out: false,
        }
    })
}

fn record_hook_failure(
    runtime: &HandlerRuntime,
    command: &str,
    hook_phase: &str,
    result: &ShellOutput,
) -> std::result::Result<(), RuntimeNodeError> {
    if result.exit_code == 0 {
        return Ok(());
    }
    if let Some(paths) = &runtime.run_paths {
        append_tool_hook_failure(
            paths,
            &runtime.node_id,
            &ToolHookFailureRecord {
                command: command.to_string(),
                exit_code: result.exit_code,
                hook_phase: hook_phase.to_string(),
                stderr: result.stderr.trim().to_string(),
                stdout: result.stdout.trim().to_string(),
            },
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
    }
    Ok(())
}

fn pre_hook_failure_reason(command: &str, result: &ShellOutput) -> String {
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return format!("tool pre-hook blocked execution: {stderr}");
    }
    format!(
        "tool pre-hook blocked execution (exit code {}): {command}",
        result.exit_code
    )
}

fn write_tool_output_if_available(
    runtime: &HandlerRuntime,
    output: &str,
) -> std::result::Result<(), RuntimeNodeError> {
    if let Some(paths) = &runtime.run_paths {
        write_tool_output_log(paths, &runtime.node_id, output)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
    }
    Ok(())
}

fn capture_declared_artifacts(
    runtime: &HandlerRuntime,
    cwd: &Path,
    stdout_text: &str,
    stderr_text: &str,
) -> std::result::Result<Option<String>, RuntimeNodeError> {
    let artifact_paths = artifact_patterns(&runtime.node_attrs, "tool.artifacts.paths");
    let stdout_path = attr_text(&runtime.node_attrs, "tool.artifacts.stdout");
    let stderr_path = attr_text(&runtime.node_attrs, "tool.artifacts.stderr");
    if artifact_paths.is_empty() && stdout_path.is_none() && stderr_path.is_none() {
        return Ok(None);
    }
    let Some(paths) = &runtime.run_paths else {
        return Ok(Some(
            "artifact capture unavailable: runtime does not expose an artifact store".to_string(),
        ));
    };
    let result = (|| -> Result<()> {
        if let Some(path) = stdout_path.as_deref() {
            write_tool_text_artifact(paths, &runtime.node_id, path, stdout_text)?;
        }
        if let Some(path) = stderr_path.as_deref() {
            write_tool_text_artifact(paths, &runtime.node_id, path, stderr_text)?;
        }
        if !artifact_paths.is_empty() {
            copy_tool_artifact_matches(paths, &runtime.node_id, cwd, &artifact_paths)?;
        }
        Ok(())
    })();
    Ok(result.err().map(|error| {
        let reason = error.to_string();
        format!("artifact capture failed: {reason}")
    }))
}

fn artifact_patterns(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> Vec<String> {
    attr_text(attrs, key)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn timeout_seconds(attrs: &BTreeMap<String, DotAttribute>) -> Option<f64> {
    attrs
        .get("timeout")
        .and_then(|attribute| match &attribute.value {
            DotValue::Duration(duration) => match duration.unit.as_str() {
                "ms" => Some(duration.value as f64 / 1000.0),
                "s" => Some(duration.value as f64),
                "m" => Some(duration.value as f64 * 60.0),
                "h" => Some(duration.value as f64 * 3600.0),
                "d" => Some(duration.value as f64 * 86400.0),
                _ => None,
            },
            DotValue::Integer(value) => Some(*value as f64),
            DotValue::Float(value) => Some(*value),
            DotValue::String(value) => value.trim().parse::<f64>().ok(),
            DotValue::Boolean(_) | DotValue::Null => None,
        })
}

fn timeout_failure_reason(command: &str, timeout: Option<Duration>) -> String {
    timeout
        .map(|timeout| {
            format!(
                "Command '{command}' timed out after {} seconds",
                timeout.as_secs_f64()
            )
        })
        .unwrap_or_else(|| "tool command timed out".to_string())
}

fn run_shell_command(
    command: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    env: BTreeMap<String, String>,
    stdin_text: Option<String>,
) -> std::io::Result<ShellOutput> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .envs(env)
        .stdin(if stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(stdin_text) = stdin_text {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(stdin_text.as_bytes());
        }
    }

    let Some(timeout) = timeout else {
        let output = child.wait_with_output()?;
        return Ok(shell_output_from_process(output, false));
    };

    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(shell_output_from_process(output, false));
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok(shell_output_from_process(output, true));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn shell_output_from_process(output: std::process::Output, timed_out: bool) -> ShellOutput {
    ShellOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        timed_out,
    }
}

fn human_skipped_outcome() -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: "human skipped interaction".to_string(),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

const HUMAN_GATE_POLL_INTERVAL_MS: u64 = 250;

enum HumanGateWaitResult {
    Answered(HumanAnswer),
    Interrupted(Outcome),
}

enum GateRunStatus {
    Waiting,
    Running,
}

fn journaled_gate_answer(paths: &RunRootPaths, question_id: &str) -> Option<String> {
    let events = crate::events::read_raw_events(paths).ok()?;
    events.iter().rev().find_map(|event| {
        if event.event_type != "InterviewCompleted" {
            return None;
        }
        if event.payload.get("question_id").and_then(Value::as_str) != Some(question_id) {
            return None;
        }
        event
            .payload
            .get("answer")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn select_choice(answer: &HumanAnswer, choices: &[Choice]) -> Option<Choice> {
    let mut tokens = answer
        .selected_values
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
        .collect::<Vec<_>>();
    if let Some(option) = &answer.selected_option {
        tokens.extend([
            option.value.trim().to_string(),
            option.label.trim().to_string(),
            option.key.trim().to_string(),
        ]);
    }
    if let Some(text) = trimmed_nonempty(&answer.text) {
        tokens.push(text);
    }
    if let Some(value) = trimmed_nonempty(&answer.value) {
        tokens.push(value);
    }
    tokens.retain(|value| !value.is_empty() && value != "skipped");

    for token in tokens {
        let normalized_token = token.to_ascii_lowercase();
        for choice in choices {
            if choice.target == token
                || choice.label == token
                || normalize_label(&choice.label) == normalize_label(&token)
                || (!choice.key.is_empty() && choice.key.eq_ignore_ascii_case(&token))
                || choice.target.to_ascii_lowercase() == normalized_token
            {
                return Some(choice.clone());
            }
        }
    }
    None
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_label(label: &str) -> String {
    let mut text = label.trim().to_ascii_lowercase();
    if text.starts_with('[') {
        if let Some(index) = text.find(']') {
            text = text[index + 1..].trim().to_string();
        }
    }
    let mut chars = text.chars();
    let first = chars.next();
    let second = chars.next();
    if matches!(second, Some(')')) && first.is_some_and(|ch| ch.is_ascii_alphanumeric()) {
        text = chars.as_str().trim().to_string();
    }
    let mut chars = text.chars();
    let first = chars.next();
    let second = chars.next();
    let third = chars.next();
    if first.is_some_and(|ch| ch.is_ascii_alphanumeric())
        && matches!(second, Some(' '))
        && matches!(third, Some('-'))
    {
        text = chars.as_str().trim().to_string();
    }
    text
}

fn parse_accelerator_key(label: &str) -> String {
    let text = label.trim();
    if text.is_empty() {
        return String::new();
    }
    if text.starts_with('[') {
        if let Some(index) = text.find(']') {
            let inside = text[1..index].trim();
            if let Some(ch) = inside.chars().next() {
                return ch.to_ascii_uppercase().to_string();
            }
        }
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() >= 2 && chars[0].is_ascii_alphanumeric() && chars[1] == ')' {
        return chars[0].to_ascii_uppercase().to_string();
    }
    if chars.len() >= 3 && chars[0].is_ascii_alphanumeric() {
        let mut index = 1;
        while index < chars.len() && chars[index] == ' ' {
            index += 1;
        }
        if index < chars.len() && chars[index] == '-' {
            return chars[0].to_ascii_uppercase().to_string();
        }
    }
    text.chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .unwrap_or_default()
}

fn validate_join_thresholds_for_config(
    join_policy: &str,
    join_k: Option<u64>,
    join_quorum: Option<f64>,
    branch_count: usize,
) -> Option<String> {
    if join_policy != "k_of_n" && join_k.is_some() {
        return Some("join_k is only supported when join_policy is k_of_n".to_string());
    }
    if join_policy != "quorum" && join_quorum.is_some() {
        return Some("join_quorum is only supported when join_policy is quorum".to_string());
    }
    if join_policy == "k_of_n" {
        let Some(join_k) = join_k else {
            return Some("join_k is required and must be an integer >= 1".to_string());
        };
        if join_k == 0 {
            return Some("join_k is required and must be an integer >= 1".to_string());
        }
        if join_k > branch_count as u64 {
            return Some(format!(
                "join_k must be <= outgoing branch count ({branch_count})"
            ));
        }
    }
    if join_policy == "quorum" {
        let join_quorum = join_quorum.unwrap_or(0.5);
        if !join_quorum.is_finite() || join_quorum <= 0.0 || join_quorum > 1.0 {
            return Some("join_quorum must be finite and > 0 and <= 1".to_string());
        }
    }
    None
}

fn fan_in_nodes(flow: &FlowDefinition) -> BTreeSet<String> {
    flow.nodes
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::FanIn)
        .map(|(node_id, _)| node_id.clone())
        .collect()
}

fn branch_payload(
    id: &str,
    status: &str,
    outcome: Option<&str>,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    node_outcomes: &BTreeMap<String, Outcome>,
    failure_reason: &str,
) -> Value {
    let node_outcomes = node_outcomes
        .iter()
        .map(|(node_id, outcome)| (node_id.clone(), json!(outcome.status.as_str())))
        .collect::<serde_json::Map<_, _>>();
    json!({
        "id": id,
        "status": status,
        "outcome": outcome.map(Value::from).unwrap_or(Value::Null),
        "outcome_reason_code": Value::Null,
        "outcome_reason_message": Value::Null,
        "current_node": current_node,
        "completed_nodes": completed_nodes,
        "context": context.snapshot(),
        "node_outcomes": node_outcomes,
        "failure_reason": failure_reason,
    })
}

fn branch_succeeded(payload: &Value) -> bool {
    object_string(payload, "status").eq_ignore_ascii_case("completed")
        && object_string(payload, "outcome").eq_ignore_ascii_case("success")
}

fn branch_failed(payload: &Value) -> bool {
    let status = object_string(payload, "status");
    let outcome = object_string(payload, "outcome");
    status.eq_ignore_ascii_case("failed")
        || (status.eq_ignore_ascii_case("completed") && outcome.eq_ignore_ascii_case("failure"))
}

fn stage_failure_reason(outcome: &Outcome) -> String {
    let reason = outcome.failure_reason.trim();
    if reason.is_empty() {
        "stage_failed".to_string()
    } else {
        reason.to_string()
    }
}

fn normalize_parallel_results(raw: &Value) -> Vec<Value> {
    if let Some(items) = raw.as_array() {
        return items
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect();
    }
    if let Some(text) = raw.as_str() {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) {
            return items.into_iter().filter(Value::is_object).collect();
        }
    }
    Vec::new()
}

fn result_status(result: &Value) -> String {
    let status = object_string(result, "status").to_ascii_lowercase();
    let outcome = object_string(result, "outcome").to_ascii_lowercase();
    if status == "completed" && !outcome.is_empty() {
        return outcome;
    }
    if status == "failed" {
        return "fail".to_string();
    }
    status
}

fn result_status_rank(status: &str) -> i32 {
    match status.to_ascii_lowercase().as_str() {
        "success" => 0,
        "partial_success" | "paused" => 1,
        "retry" => 2,
        "fail" => 3,
        _ => 4,
    }
}

fn compare_fan_in_candidates(left: &Value, right: &Value) -> Ordering {
    result_status_rank(&result_status(left))
        .cmp(&result_status_rank(&result_status(right)))
        .then_with(|| {
            score_value(right)
                .partial_cmp(&score_value(left))
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| object_string(left, "id").cmp(&object_string(right, "id")))
}

fn score_value(result: &Value) -> f64 {
    let Some(score) = result.get("score") else {
        return 0.0;
    };
    score
        .as_f64()
        .or_else(|| score.as_str().and_then(|value| value.parse::<f64>().ok()))
        .unwrap_or(0.0)
}

fn extract_best_id(text: &str) -> Option<String> {
    let raw = text.trim();
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(object)) => object
            .get("best_id")
            .or_else(|| object.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        Ok(Value::String(value)) => trimmed_nonempty(&value),
        _ => Some(raw.to_string()),
    }
}

fn object_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}
