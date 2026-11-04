use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use attractor_core::{
    attr_text, AttractorContext, BranchTerminal, ContextMap, DotAttribute, DotGraph, DotValue,
    FailureKind, FlowDefinition, FlowNode, NodeConfig, Outcome, RoutingEdge,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_agent_adapter::codergen::CodergenEvent;
use spark_common::debug::{agent_trace_enabled, AGENT_TRACE_FILE_NAME};
use spark_storage::append_jsonl;

use crate::artifacts::{
    append_tool_hook_failure, copy_tool_artifact_matches, write_tool_output_log,
    write_tool_text_artifact, ToolHookFailureRecord,
};
use crate::codergen::{codergen_outcome, RuntimeCodergen};
use crate::context::{clear_runtime_retry_context, seed_builtin_context};
use crate::error::Result;
use crate::events::{
    append_event, interview_completed_event, interview_started_event,
    parallel_branch_completed_event, parallel_branch_node_completed_event,
    parallel_branch_node_started_event, parallel_branch_started_event, parallel_completed_event,
    parallel_started_event,
};
use crate::executor::{
    outcome_from_node_error, panic_payload_message, NodeExecutionRequest, NodeExecutor,
    RuntimeNodeError,
};
use crate::paths::RunRootPaths;
use crate::retry::{retry_policy_for_outcome, should_retry_attempt};
use crate::routing::{
    is_exit_node, mark_unavailable_routes, outgoing_routing_edges, select_transition,
    stage_failure_reason,
};

pub const HANDLER_START: &str = "start";
pub const HANDLER_EXIT: &str = "exit";
pub const HANDLER_CODERGEN: &str = "codergen";
pub const HANDLER_WAIT_HUMAN: &str = "wait.human";
pub const HANDLER_SWITCH: &str = "switch";
pub const HANDLER_PARALLEL: &str = "parallel";
pub const HANDLER_FAN_IN: &str = "parallel.fan_in";
pub const HANDLER_TOOL: &str = "tool";
pub const HANDLER_MANAGER_LOOP: &str = "stack.manager_loop";

fn materialize_execution_suffix(
    activity: &spark_storage::ActivityRepository,
) -> spark_storage::Result<()> {
    let records = activity.read_transcript_records()?;
    let mut revision = records.len() as u64 + 1;
    // Messages sent into the turn, so one it never read is marked failed
    // where it was sent, as chat shows it.
    let mut user_messages = records
        .into_iter()
        .filter_map(|record| match record {
            spark_storage::TranscriptRecord::SegmentUpsert { segment, .. }
                if segment.kind == "user_message" =>
            {
                Some((segment.id.clone(), segment))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for stored in activity.uncommitted_event_suffix()? {
        if stored.event.get("type").and_then(Value::as_str) == Some("prompt_resolved") {
            let prompt = stored
                .event
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let turn = serde_json::from_value(json!({
                "id": "prompt", "role": "user", "kind": "message", "content": prompt,
                "status": "complete", "timestamp": stored.committed_at,
            }))
            .map_err(|source| spark_storage::StorageError::JsonWrite {
                path: activity.transcript_path(),
                source,
            })?;
            activity.append_transcript(&spark_storage::TranscriptRecord::TurnUpsert {
                revision,
                committed_at: stored.committed_at,
                source_event_sequence: stored.sequence,
                turn,
            })?;
            revision += 1;
            continue;
        }
        let Some(unit) = stored
            .event
            .get("payload")
            .and_then(|payload| payload.get("turn_stream_event"))
        else {
            continue;
        };
        let kind = unit.get("kind").and_then(Value::as_str).unwrap_or_default();
        if !matches!(
            kind,
            "content_completed"
                | "tool_call_completed"
                | "tool_call_failed"
                | "request_user_input_requested"
                | "context_compaction_completed"
                | "warning"
                | "user_message_delivered"
                | "user_message_not_delivered"
        ) {
            continue;
        }
        let channel = unit
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or("assistant");
        let segment_kind = match kind {
            "tool_call_completed" | "tool_call_failed" => "tool_call",
            "request_user_input_requested" => "request_user_input",
            "context_compaction_completed" => "context_compaction",
            "warning" => "agent_event",
            "user_message_delivered" | "user_message_not_delivered" => "user_message",
            _ => match channel {
                "reasoning" => "reasoning",
                "plan" => "plan",
                _ => "assistant_message",
            },
        };
        let id = if segment_kind == "user_message" {
            format!(
                "segment-user-message-{}",
                unit["source"]["item_id"].as_str().unwrap_or_default()
            )
        } else {
            unit.get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("event-{}", stored.sequence))
        };
        let segment = if kind == "user_message_not_delivered" {
            let Some(mut segment) = user_messages.get(&id).cloned() else {
                continue;
            };
            segment.status = "failed".to_string();
            segment.error = unit
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            segment.updated_at = stored.committed_at.clone();
            segment
        } else {
            let mut segment = json!({
                "id": id, "turn_id": "response", "order": revision as i64 - 2,
                "kind": segment_kind,
                "role": if segment_kind == "user_message" { "user" } else { "assistant" },
                "status": if kind == "tool_call_failed" { "failed" } else { "complete" },
                "timestamp": stored.committed_at, "updated_at": stored.committed_at,
                "completed_at": stored.committed_at,
                "content": unit.get("message").or_else(|| unit.get("content")).and_then(Value::as_str).unwrap_or_default(),
                "tool_call": unit.get("tool_call"), "request_user_input": unit.get("request_user_input"),
                "source": unit.get("source"),
            });
            if kind == "warning" {
                segment["category"] = json!("warning");
            }
            serde_json::from_value(segment).map_err(|source| {
                spark_storage::StorageError::JsonWrite {
                    path: activity.transcript_path(),
                    source,
                }
            })?
        };
        if segment_kind == "user_message" {
            user_messages.insert(id, segment.clone());
        }
        activity.append_transcript(&spark_storage::TranscriptRecord::SegmentUpsert {
            revision,
            committed_at: stored.committed_at,
            source_event_sequence: stored.sequence,
            segment,
        })?;
        revision += 1;
    }
    Ok(())
}

fn externalize_codergen_tool_output(
    activity: &spark_storage::ActivityRepository,
    event: &mut CodergenEvent,
) -> spark_storage::Result<()> {
    let kind = event
        .payload
        .get("turn_stream_event")
        .and_then(|event| event.get("kind"))
        .or_else(|| event.payload.get("kind"))
        .and_then(Value::as_str);
    if !matches!(kind, Some("tool_call_completed" | "tool_call_failed")) {
        return Ok(());
    }
    let normalized_id = event
        .payload
        .get("turn_stream_event")
        .and_then(|event| event.get("tool_call"))
        .and_then(spark_common::segments::tool_call_id);
    if let Some(id) = normalized_id {
        let bounded = {
            let tool_call = event
                .payload
                .get_mut("turn_stream_event")
                .and_then(Value::as_object_mut)
                .and_then(|event| event.get_mut("tool_call"));
            let Some(tool_call) = tool_call else {
                return Ok(());
            };
            activity.externalize_tool_output(&id, tool_call)?;
            tool_call.clone()
        };
        if let Some(tool_event) = event
            .payload
            .get_mut("tool_event")
            .filter(|tool_event| {
                spark_common::segments::tool_call_id(tool_event).as_deref() == Some(id.as_str())
            })
            .and_then(Value::as_object_mut)
        {
            for key in [
                "output",
                "output_size",
                "output_truncated",
                "output_artifact",
            ] {
                if let Some(value) = bounded.get(key) {
                    tool_event.insert(key.to_string(), value.clone());
                }
            }
        }
        return Ok(());
    }
    let Some(tool_event) = event.payload.get_mut("tool_event") else {
        return Ok(());
    };
    let Some(id) = spark_common::segments::tool_call_id(tool_event) else {
        return Ok(());
    };
    activity.externalize_tool_output(&id, tool_event)
}

pub type RuntimeHandlerFn =
    Box<dyn FnMut(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError> + Send>;

pub type RuntimeThreadSafeHandlerFn =
    dyn Fn(HandlerRuntime) -> std::result::Result<Outcome, RuntimeNodeError> + Send + Sync;

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

/// Sessions held by running nodes, by run id and session name, so two
/// nodes never drive one provider session at once.
static SESSIONS_IN_USE: Mutex<BTreeMap<(String, String), String>> = Mutex::new(BTreeMap::new());

struct SessionLease((String, String));

impl SessionLease {
    /// Holds `name` for `node_id`, or names the node already holding it.
    fn acquire(run_id: &str, name: &str, node_id: &str) -> std::result::Result<Self, String> {
        let key = (run_id.to_string(), name.to_string());
        let mut in_use = SESSIONS_IN_USE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(holder) = in_use.get(&key) {
            return Err(holder.clone());
        }
        in_use.insert(key.clone(), node_id.to_string());
        Ok(Self(key))
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        SESSIONS_IN_USE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&self.0);
    }
}

#[derive(Debug, Clone)]
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
    pub attempt: u64,
    pub run_paths: Option<RunRootPaths>,
    pub fallback_model: Option<String>,
    pub fallback_provider: Option<String>,
    pub fallback_profile: Option<String>,
    pub fallback_reasoning_effort: Option<String>,
    /// Set by the owning fork when this node runs in a branch: raised when
    /// the fork stops its remaining branches.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Raised when this invocation was actually interrupted by its fork's
    /// cancellation or the run's, so its error is the cancellation and not
    /// an execution error of its own.
    pub interrupted: Arc<AtomicBool>,
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
            attempt: request.attempt,
            run_paths: request.run_paths,
            fallback_model: request.fallback_model,
            fallback_provider: request.fallback_provider,
            fallback_profile: request.fallback_profile,
            fallback_reasoning_effort: request.fallback_reasoning_effort,
            cancel: None,
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    fn canceled_by_owner(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::SeqCst))
    }

    /// Whether the owning fork or the user canceled this invocation; a
    /// positive answer marks the invocation interrupted.
    pub(crate) fn stop_requested(&self) -> bool {
        let stop = self.canceled_by_owner()
            || self
                .run_paths
                .as_ref()
                .is_some_and(|paths| run_record_cancel_requested(paths));
        if stop {
            self.interrupted.store(true, Ordering::SeqCst);
        }
        stop
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
    /// Free-text note the human attached to their selection; propagated to
    /// downstream nodes as `human.gate.note`, never used for route matching.
    #[serde(default)]
    pub note: String,
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
pub struct ChildRunRequest {
    pub child_run_id: String,
    pub child_flow: FlowDefinition,
    pub child_flow_name: String,
    /// The child's flow name, resolved from its `flow_ref`.
    pub child_flow_ref: String,
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
    /// A failed child's error class, from its run record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
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
    /// The child's context at its last checkpoint, for `output_map` and the
    /// sessions it hands back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: ContextMap,
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

fn write_agent_trace_event(trace_path: Option<&std::path::Path>, event: &CodergenEvent) {
    if !should_write_agent_trace(event) {
        return;
    }
    let Some(trace_path) = trace_path else {
        return;
    };
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

fn should_write_agent_trace(event: &CodergenEvent) -> bool {
    matches!(
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
    codergen_backend_factory: Option<CodergenBackendFactory>,
    rust_llm_client: Option<unified_llm_adapter::Client>,
    codergen_intervention_broker: Option<spark_agent_adapter::CodergenSessionInterventionBroker>,
    event_append_lock: Arc<Mutex<()>>,
    child_run_launcher: Option<ChildRunLauncher>,
    child_status_resolver: Option<ChildStatusResolver>,
    child_intervention_requester: Option<ChildInterventionRequester>,
    run_event_observer: Option<crate::store::RunEventObserver>,
    external_event_sink:
        Option<Arc<dyn Fn(attractor_core::RawRuntimeEvent) -> Result<()> + Send + Sync>>,
    human_gate_blocking: bool,
}

struct BranchCompletion {
    record: Value,
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
            codergen_backend_factory: None,
            rust_llm_client: None,
            codergen_intervention_broker: None,
            event_append_lock: Arc::new(Mutex::new(())),
            child_run_launcher: None,
            child_status_resolver: None,
            child_intervention_requester: None,
            run_event_observer: None,
            external_event_sink: None,
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

    pub fn with_external_event_sink(
        mut self,
        sink: impl Fn(attractor_core::RawRuntimeEvent) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.external_event_sink = Some(Arc::new(sink));
        self
    }

    pub fn set_external_event_sink(
        &mut self,
        sink: impl Fn(attractor_core::RawRuntimeEvent) -> Result<()> + Send + Sync + 'static,
    ) {
        self.external_event_sink = Some(Arc::new(sink));
    }

    pub fn append_and_notify(
        &self,
        paths: &RunRootPaths,
        event: attractor_core::RawRuntimeEvent,
    ) -> Result<attractor_core::RawRuntimeEvent> {
        let _guard = self.event_append_lock.lock().map_err(|_| {
            crate::error::RuntimeStorageError::io(
                "lock runtime event append",
                paths.events_jsonl(),
                std::io::Error::other("runtime event append lock poisoned"),
            )
        })?;
        let event = append_event(paths, event)?;
        if let Some(observer) = &self.run_event_observer {
            observer(&paths.run_id);
        }
        Ok(event)
    }

    pub fn notify_run_event(&self, run_id: &str) {
        if let Some(observer) = &self.run_event_observer {
            observer(run_id);
        }
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

    pub fn with_codergen_backend_factory(
        mut self,
        factory: impl Fn() -> Box<dyn spark_agent_adapter::CodergenBackend> + Send + Sync + 'static,
    ) -> Self {
        self.rust_llm_client = None;
        self.codergen_backend_factory = Some(Arc::new(factory));
        self.codergen_intervention_broker = None;
        self
    }

    pub fn with_rust_llm_client(mut self, client: unified_llm_adapter::Client) -> Self {
        self.rust_llm_client = Some(client.clone());
        let broker = spark_agent_adapter::CodergenSessionInterventionBroker::shared();
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
        self.rust_llm_client = None;
        self.codergen_backend_factory = Some(Arc::new(factory));
        self.codergen_intervention_broker = None;
    }

    pub fn set_rust_llm_client(&mut self, client: unified_llm_adapter::Client) {
        self.rust_llm_client = Some(client.clone());
        let broker = spark_agent_adapter::CodergenSessionInterventionBroker::shared();
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

    pub fn child_run_launcher(&self) -> Option<ChildRunLauncher> {
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
            HANDLER_START => Ok(Outcome::completed(attractor_core::NEXT_DECISION)),
            HANDLER_EXIT => Ok(Outcome::completed("")),
            HANDLER_SWITCH => Ok(execute_switch(&runtime)),
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
        // A container worker shares the host's run root: journal there so
        // the host sees the event, and forward it so the host is notified.
        let event = match &runtime.run_paths {
            Some(paths) => self.append_and_notify(paths, event)?,
            None => event,
        };
        if let Some(sink) = &self.external_event_sink {
            sink(event)?;
        }
        Ok(())
    }

    pub(crate) fn emit_transcript_event(
        &self,
        runtime: &HandlerRuntime,
        event: attractor_core::RawRuntimeEvent,
    ) -> Result<()> {
        let event = if let Some(paths) = &runtime.run_paths {
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
            append_event(paths, event)?
        } else {
            event
        };
        if let Some(sink) = &self.external_event_sink {
            sink(event)?;
        }
        Ok(())
    }

    fn execute_codergen(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let execution_root = runtime.run_paths.as_ref().map(|paths| {
            paths
                .logs_dir()
                .join(&runtime.node_id)
                .join("executions")
                .join(format!("{}-{}", runtime.stage_index, runtime.attempt))
        });
        let activity = execution_root
            .as_ref()
            .map(spark_storage::ActivityRepository::new);
        if let Some(activity) = activity.as_ref() {
            if activity
                .read_transcript_records()
                .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?
                .is_empty()
            {
                if activity
                    .read_events()
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?
                    .is_empty()
                {
                    activity
                        .append_event(
                            json!({"type": "prompt_resolved", "prompt": runtime.prompt}),
                            crate::events::utc_timestamp(),
                        )
                        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
                }
                materialize_execution_suffix(activity)
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            }
        }
        let root_run_id = runtime
            .context
            .get("internal.root_run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(runtime.run_id.as_str())
            .to_string();
        let workdir_key = runtime
            .node
            .runtime
            .as_ref()
            .and_then(|config| config.workdir_from.as_deref())
            .map(str::trim)
            .filter(|key| !key.is_empty());
        let agent_workdir = match workdir_key {
            Some(key) => match crate::manager_loop::resolve_context_workdir(
                &runtime.context,
                &runtime.run_workdir,
                key,
                "runtime.workdir_from",
            ) {
                Ok(path) => path,
                Err(reason) => {
                    return Ok(Outcome::error(FailureKind::Runtime, reason, Some(false)));
                }
            },
            None => runtime.run_workdir.clone(),
        };
        let session_name = runtime
            .node
            .runtime
            .as_ref()
            .and_then(|config| config.session.clone());
        let _session_lease = match session_name.as_deref() {
            Some(name) => match SessionLease::acquire(&runtime.run_id, name, &runtime.node_id) {
                Ok(lease) => Some(lease),
                Err(holder) => {
                    return Ok(Outcome::error(
                        FailureKind::Runtime,
                        format!("session '{name}' is already in use by running node '{holder}'"),
                        Some(false),
                    ));
                }
            },
            None => None,
        };
        // A child run's sessions belong to its parent's subflow node.
        let session_scope = runtime
            .context
            .get("internal.parent_node_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("run")
            .to_string();
        let session = session_name
            .clone()
            .map(|name| spark_agent_adapter::CodergenSession {
                handle: crate::context::session_entry(&runtime.context, "named", &name)
                    .and_then(|value| serde_json::from_value(value).ok()),
                history_path: runtime.run_paths.as_ref().map(|paths| {
                    paths
                        .root
                        .join("sessions")
                        .join(&session_scope)
                        .join(format!("{name}.json"))
                }),
                name,
            });
        let clarification_interrupted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut codergen = if let Some(factory) = self.codergen_backend_factory.as_ref() {
            RuntimeCodergen::with_boxed_backend(
                runtime.handler_graph.clone(),
                runtime.logs_root.clone(),
                {
                    let mut backend = match (
                        self.codergen_intervention_broker.as_ref().zip(self.rust_llm_client.as_ref()),
                        runtime.context.get("internal.llm_profiles_snapshot"),
                    ) {
                        (Some((broker, base_client)), Some(value)) => {
                            let profiles = serde_json::from_value(value.clone())
                                .map_err(|_| RuntimeNodeError::runtime("Invalid captured LLM profiles."))?;
                            let env = std::env::vars().collect::<BTreeMap<_, _>>();
                            let client = if let Some(value) = runtime.context.get("internal.execution_configuration_snapshot") {
                                let configuration: spark_storage::settings::ExecutionConfiguration = serde_json::from_value(value.clone()).map_err(|_| RuntimeNodeError::runtime("Invalid captured execution configuration."))?;
                                base_client.clone().with_execution_environment(&configuration.providers.execution_environment(&env)).map_err(|_| RuntimeNodeError::runtime("Unable to initialize captured provider connections."))?
                            } else { base_client.clone() };
                            let client = client.with_profile_definitions(profiles, &env)
                                .map_err(|_| RuntimeNodeError::runtime("Unable to initialize captured LLM profiles; check credential environment references."))?;
                            Box::new(spark_agent_adapter::RustLlmCodergenBackend::with_intervention_broker(client, broker.clone()))
                                as Box<dyn spark_agent_adapter::CodergenBackend>
                        }
                        _ => factory(),
                    };
                    let text_only = runtime
                        .node
                        .contracts
                        .as_ref()
                        .and_then(|contracts| contracts.runtime_mode.as_deref())
                        .is_some_and(|mode| {
                            mode.trim().to_ascii_lowercase().replace('-', "_") == "text_only"
                        });
                    if runtime.run_paths.is_some() || runtime.cancel.is_some() {
                        let check_runtime = runtime.clone();
                        backend.set_cancel_check(Arc::new(move || check_runtime.stop_requested()));
                    }
                    if runtime.run_paths.is_some() && !text_only {
                        let runner = self.clone();
                        let question_runtime = runtime.clone();
                        let interrupted = clarification_interrupted.clone();
                        backend.set_clarification_handler(Arc::new(move |params| {
                            runner.start_agent_clarification(
                                &question_runtime,
                                params,
                                interrupted.clone(),
                            )
                        }));
                    }
                    backend
                },
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
        .with_runtime_context(Some(agent_workdir), {
            let mut metadata = BTreeMap::from([
                (
                    "spark.runtime.run_id".to_string(),
                    json!(runtime.run_id.clone()),
                ),
                ("spark.runtime.root_run_id".to_string(), json!(root_run_id)),
                (
                    "spark.runtime.run_workdir".to_string(),
                    json!(runtime.run_workdir.to_string_lossy().to_string()),
                ),
            ]);
            if let Some(configuration) = runtime.context.get("internal.execution_configuration_snapshot") {
                metadata.insert("spark.execution.settings".into(), json!({"configuration": configuration, "model_settings": runtime.context.get("internal.model_defaults_snapshot").and_then(|v| v.get("reasoning_controls").or_else(|| v.get("group")))}));
            }
            if let Some(root) = execution_root.as_ref() {
                metadata.insert("spark.runtime.execution_root".to_string(), json!(root));
            }
            metadata
        })
        .with_session(session);
        // Persist detailed codergen events in the owning execution while the
        // node runs, then materialize completed logical units into its transcript.
        let live_sink_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let trace_enabled = runtime
            .context
            .get("internal.execution_configuration_snapshot")
            .and_then(|value| value["agents"]["native"]["agent_trace"].as_bool())
            .unwrap_or_else(agent_trace_enabled);
        let trace_path = execution_root
            .as_ref()
            .filter(|_| trace_enabled)
            .map(|root| root.join(AGENT_TRACE_FILE_NAME));
        let has_event_destination =
            self.external_event_sink.is_some() || runtime.run_paths.is_some();
        let live_sink = has_event_destination.then(|| {
            let run_id = runtime.run_id.clone();
            let node_id = runtime.node_id.clone();
            let runner = self.clone();
            let sink_error = Arc::clone(&live_sink_error);
            let trace_path = trace_path.clone();
            let activity = activity.clone();
            let sink_runtime = runtime.clone();
            Arc::new(move |mut event: spark_agent_adapter::CodergenEvent| {
                write_agent_trace_event(trace_path.as_deref(), &event);
                if let Some(activity) = activity.as_ref() {
                    if let Err(error) = externalize_codergen_tool_output(activity, &mut event) {
                        let mut slot = sink_error
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        if slot.is_none() {
                            *slot = Some(error.to_string());
                        }
                        return;
                    }
                }
                let value = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
                let append_result = if let Some(activity) = activity.as_ref() {
                    activity
                        .append_event(value, crate::events::utc_timestamp())
                        .and_then(|_| materialize_execution_suffix(activity))
                        .map_err(
                            |error| crate::error::RuntimeStorageError::InvalidRuntimeGraph {
                                reason: error.to_string(),
                            },
                        )
                } else {
                    let raw_event = crate::events::codergen_adapter_event(
                        &run_id,
                        &node_id,
                        &event.event_type,
                        serde_json::to_value(&event.payload).unwrap_or_else(|_| json!({})),
                    );
                    runner.emit(&sink_runtime, raw_event)
                };
                // Published only: the execution activity is its durable record.
                let publish_result = if let Some(sink) = runner
                    .external_event_sink
                    .as_ref()
                    .filter(|_| activity.is_some())
                {
                    sink(crate::events::codergen_adapter_event(
                        &run_id,
                        &node_id,
                        &event.event_type,
                        serde_json::to_value(&event.payload).unwrap_or_else(|_| json!({})),
                    ))
                } else {
                    Ok(())
                };
                match append_result.and(publish_result) {
                    Ok(()) => {
                        if activity.is_some() {
                            if let Some(observer) = &runner.run_event_observer {
                                observer(&run_id);
                            }
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
            }) as spark_agent_adapter::CodergenEventSink
        });
        let execution = codergen
            .execute_with_event_sink(&runtime.node_id, runtime.context.clone(), live_sink)
            .map_err(|error| {
                if clarification_interrupted.load(std::sync::atomic::Ordering::Relaxed) {
                    RuntimeNodeError::terminal(error.to_string())
                } else {
                    RuntimeNodeError::runtime(error.to_string())
                }
            })?;
        if let Some(error) = live_sink_error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            return Err(RuntimeNodeError::runtime(error));
        }
        if let Some(activity) = activity.as_ref() {
            materialize_execution_suffix(activity)
                .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        }
        // Text-only completions never stream a content event, so the final
        // response text would otherwise be invisible to the execution
        // transcript. Journal a synthetic completion for those.
        let streamed_assistant_content = execution.events.iter().any(|event| {
            event
                .payload
                .get("turn_stream_event")
                .and_then(|value| value.get("kind"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| matches!(kind, "content_delta" | "content_completed"))
        });
        if has_event_destination
            && !streamed_assistant_content
            && !execution.response_text.trim().is_empty()
        {
            if let Some(activity) = activity.as_ref() {
                let committed_at = crate::events::utc_timestamp();
                let event = activity
                    .append_event(
                        json!({"event_type": "final_response_text", "payload": {
                            "turn_stream_event": {"kind": "content_completed", "channel": "assistant",
                            "message": execution.response_text}
                        }}),
                        committed_at.clone(),
                    )
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
                let revision = activity
                    .read_transcript_records()
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?
                    .len() as u64
                    + 1;
                activity
                    .append_transcript(&spark_storage::TranscriptRecord::SegmentUpsert {
                        revision,
                        committed_at: committed_at.clone(),
                        source_event_sequence: event.sequence,
                        segment: serde_json::from_value(json!({
                            "id": "final-response", "turn_id": "response", "order": revision as i64,
                            "kind": "assistant_message", "role": "assistant", "status": "complete",
                            "timestamp": committed_at, "updated_at": committed_at,
                            "completed_at": committed_at, "content": execution.response_text,
                        }))
                        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?,
                    })
                    .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
            }
        }
        let handle = execution.session.clone();
        let mut outcome = codergen_outcome(execution);
        if let Some((name, handle)) = session_name.zip(handle) {
            outcome.context_updates.insert(
                crate::context::RUNTIME_SESSIONS_KEY.to_string(),
                crate::context::sessions_with_entry(
                    &runtime.context,
                    "named",
                    &name,
                    serde_json::to_value(handle).unwrap_or_default(),
                ),
            );
        }
        Ok(outcome)
    }

    fn execute_tool(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let Some(command) = attr_text(&runtime.node_attrs, "tool.command") else {
            return Ok(Outcome::error(
                FailureKind::Runtime,
                "No tool.command specified",
                Some(false),
            ));
        };
        let cwd = tool_cwd(&runtime);
        let tool_env = match resolve_tool_env_map(&runtime) {
            Ok(env) => env,
            Err(reason) => {
                write_tool_output_if_available(&runtime, "")?;
                return Ok(Outcome {
                    context_updates: tool_context_updates("", -1),
                    ..Outcome::error(FailureKind::Contract, reason, Some(false))
                });
            }
        };

        if let Some(pre_hook) = resolve_hook_command(&runtime, "tool.hooks.pre") {
            let pre_hook_result = run_hook(&pre_hook, "pre", &runtime, &command, &cwd);
            record_hook_failure(&runtime, &pre_hook, "pre", &pre_hook_result)?;
            if pre_hook_result.exit_code != 0 {
                write_tool_output_if_available(&runtime, "")?;
                return Ok(Outcome {
                    context_updates: tool_context_updates("", -1),
                    ..Outcome::error(
                        FailureKind::Runtime,
                        pre_hook_failure_reason(&pre_hook, &pre_hook_result),
                        None,
                    )
                });
            }
        }

        let timeout = timeout_seconds(&runtime.node_attrs).map(Duration::from_secs_f64);
        // Branch tools stop when their fork or the user cancels; the run
        // record is read at most every 250 ms.
        let mut last_check = Instant::now();
        let mut stop_check = || {
            if runtime.canceled_by_owner() {
                runtime.interrupted.store(true, Ordering::SeqCst);
                return true;
            }
            if last_check.elapsed() < Duration::from_millis(250) {
                return false;
            }
            last_check = Instant::now();
            runtime.stop_requested()
        };
        let spawned = run_shell_command(
            &command,
            &cwd,
            timeout,
            tool_env,
            None,
            runtime
                .cancel
                .is_some()
                .then_some(&mut stop_check as &mut dyn FnMut() -> bool),
        );
        let command_output = match spawned {
            Ok(output) => output,
            Err(error) => {
                write_tool_output_if_available(&runtime, "")?;
                return Ok(Outcome {
                    context_updates: tool_context_updates("", -1),
                    ..Outcome::error(
                        FailureKind::Runtime,
                        format!("tool command could not be launched: {error}"),
                        None,
                    )
                });
            }
        };
        write_tool_output_if_available(&runtime, &command_output.stdout)?;

        let mut outcome = if command_output.interrupted {
            Outcome {
                context_updates: tool_context_updates(command_output.stdout.trim(), -1),
                ..Outcome::error(
                    FailureKind::Runtime,
                    "tool command stopped by cancellation",
                    Some(false),
                )
            }
        } else if command_output.timed_out {
            Outcome {
                context_updates: tool_context_updates(command_output.stdout.trim(), -1),
                ..Outcome::error(
                    FailureKind::Runtime,
                    timeout_failure_reason(&command, timeout),
                    None,
                )
            }
        } else {
            tool_decision_outcome(&runtime, &command_output)
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
            outcome = Outcome {
                context_updates: outcome.context_updates,
                ..Outcome::error(FailureKind::Runtime, reason, Some(false))
            };
        }
        Ok(outcome)
    }

    fn start_agent_clarification(
        &self,
        runtime: &HandlerRuntime,
        params: Value,
        interrupted: Arc<std::sync::atomic::AtomicBool>,
    ) -> std::result::Result<spark_agent_adapter::codergen::ClarificationPoll, String> {
        let paths = runtime
            .run_paths
            .clone()
            .ok_or("Clarification requires run storage")?;
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .filter(|questions| !questions.is_empty())
            .ok_or("Clarification requires questions")?;
        let mut ids = std::collections::BTreeSet::new();
        for question in questions {
            let id = question
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or("Question requires an ID")?;
            if !ids.insert(id) {
                return Err("Duplicate question ID".into());
            }
            if question
                .get("question")
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err("Question requires a prompt".into());
            }
        }
        let active = crate::clarification::ActiveClarification::new(&paths)?;
        let mut question_ids = Vec::new();
        self.write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Waiting)
            .map_err(|e| e.message)?;
        for (index, question) in questions.iter().enumerate() {
            let original_id = question["id"].as_str().unwrap().to_string();
            let question_id = format!("clarification-{}-{index}", active.id);
            let options = question.get("options").and_then(Value::as_array).into_iter().flatten()
                .map(|option| json!({"label": option["label"], "value": option["label"], "description": option["description"]})).collect();
            let mut event = crate::events::human_gate_pending_event(
                &runtime.run_id,
                &question_id,
                &runtime.node_id,
                "",
                question["question"].as_str().unwrap(),
                None,
                options,
            );
            event.payload.extend(BTreeMap::from([
                ("origin".into(), json!("agent_clarification")),
                ("clarification_id".into(), json!(active.id)),
                (
                    "agent_request_id".into(),
                    params["agent_request_id"].clone(),
                ),
                ("agent_question_id".into(), json!(original_id)),
                ("stage_index".into(), json!(runtime.stage_index)),
                ("attempt".into(), json!(runtime.attempt)),
                ("question_type".into(), json!("FREEFORM")),
            ]));
            self.emit(runtime, event).map_err(|e| e.to_string())?;
            question_ids.push((question_id, original_id));
        }
        let runner = self.clone();
        let runtime = runtime.clone();
        Ok(Box::new(move || {
            let _keep_alive = &active;
            let status = crate::records::read_run_record(&paths)
                .map_err(|e| e.to_string())?
                .map(|r| crate::records::normalize_run_status(&r.status))
                .unwrap_or_default();
            if !matches!(status.as_str(), "waiting" | "running") {
                interrupted.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(format!("Run {status} while waiting for clarification"));
            }
            let events = crate::events::read_raw_events(&paths).map_err(|e| e.to_string())?;
            let answers: BTreeMap<String, String> = question_ids
                .iter()
                .filter_map(|(id, original)| {
                    gate_answer_in(events.iter(), id).map(|(answer, _)| (original.clone(), answer))
                })
                .collect();
            if answers.len() != question_ids.len() {
                return Ok(None);
            }
            let still_waiting = events.iter().any(|event| {
                event.event_type == "human_gate"
                    && event.payload.get("origin").and_then(Value::as_str)
                        == Some("agent_clarification")
                    && event
                        .payload
                        .get("question_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| gate_answer_in(events.iter(), id).is_none())
                    && crate::clarification::is_active(&paths, &json!(event.payload))
            });
            if !still_waiting {
                runner
                    .write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Running)
                    .map_err(|e| e.message)?;
            }
            Ok(Some(answers))
        }))
    }

    fn execute_human(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        // The choices are the gate's declared decisions; labels are display.
        let choices = runtime
            .outgoing_edges
            .iter()
            .map(|edge| {
                let label = edge.display_label().to_string();
                Choice {
                    key: parse_accelerator_key(&label),
                    label,
                    decision: edge.decision.clone(),
                }
            })
            .collect::<Vec<_>>();
        let question_text = if runtime.prompt.trim().is_empty() {
            "Choose next route".to_string()
        } else {
            runtime.prompt.clone()
        };
        let details = human_gate_details(&runtime);
        let question = HumanQuestion {
            text: question_text.clone(),
            stage: runtime.node_id.clone(),
            options: choices.iter().map(Choice::option).collect(),
        };

        self.emit(
            &runtime,
            interview_started_event(&runtime.run_id, &runtime.node_id, &question_text),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        let answer = self
            .interviewer
            .lock()
            .map_err(|_| RuntimeNodeError::runtime("interviewer lock poisoned"))?
            .ask(question);
        let mut selected = select_choice(&answer, &choices).map(|choice| (choice, answer.note));
        // Absent, skipped or invalid answers leave the gate waiting.
        if selected.is_none() && self.human_gate_blocking && runtime.run_paths.is_some() {
            match self.wait_for_human_gate_answer(&runtime, &question_text, details, &choices)? {
                HumanGateWaitResult::Answered(choice, note) => selected = Some((choice, note)),
                HumanGateWaitResult::Interrupted(outcome) => return Ok(outcome),
            }
        }
        let Some((selected, note)) = selected else {
            // Nothing can deliver an answer to this gate: an execution
            // error, never a decision.
            return Ok(Outcome::error(
                FailureKind::Runtime,
                "human gate received no valid answer and cannot wait for one",
                Some(false),
            ));
        };
        self.emit_transcript_event(
            &runtime,
            interview_completed_event(
                &runtime.run_id,
                &runtime.node_id,
                &question_text,
                selected.label.clone(),
                "accepted",
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        // Always write the note key so a note from an earlier gate cannot
        // leak past a later gate answered without one.
        let context_updates = ContextMap::from([
            ("human.gate.selected".to_string(), json!(selected.decision)),
            ("human.gate.label".to_string(), json!(selected.label)),
            ("human.gate.note".to_string(), json!(note.trim())),
        ]);
        Ok(Outcome {
            context_updates,
            notes: note.trim().to_string(),
            ..Outcome::completed(selected.decision)
        })
    }

    /// Publishes the pending question, marks the run `waiting`, and polls the
    /// journal until the answer route records an InterviewCompleted for this
    /// question — or a persisted cancel/pause request interrupts the wait.
    fn wait_for_human_gate_answer(
        &self,
        runtime: &HandlerRuntime,
        question_text: &str,
        details: Option<String>,
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
            .map(|choice| json!(choice.option()))
            .collect::<Vec<_>>();
        self.emit(
            runtime,
            crate::events::human_gate_pending_event(
                &runtime.run_id,
                &question_id,
                &runtime.node_id,
                &flow_name,
                question_text,
                details,
                options,
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
        self.write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Waiting)?;

        // Incremental answer scan: the first pass reads the whole journal
        // (a resumed gate may find its answer already recorded); afterwards
        // each 250 ms poll parses only bytes appended since the last poll,
        // so an unanswered gate costs nothing while it blocks — and gates
        // block until answered, by design.
        let mut scan_offset: Option<u64> = None;
        loop {
            // Answers naming no declared decision are rejected and skipped;
            // the gate keeps waiting.
            let answer = match scan_offset {
                None => {
                    scan_offset = Some(events_length(&paths));
                    crate::events::read_raw_events(&paths)
                        .ok()
                        .and_then(|events| {
                            valid_gate_answer(events.iter().rev(), &question_id, choices)
                        })
                }
                Some(offset) => match crate::events::read_raw_events_after(&paths, offset) {
                    Ok(Some((appended, consumed))) => {
                        scan_offset = Some(consumed);
                        valid_gate_answer(appended.iter(), &question_id, choices)
                    }
                    // Shrunk or unreadable log: rescan from scratch next poll.
                    _ => {
                        scan_offset = None;
                        None
                    }
                },
            };
            if let Some((choice, note)) = answer {
                self.write_gate_run_status(&paths, &runtime.run_id, GateRunStatus::Running)?;
                return Ok(HumanGateWaitResult::Answered(choice, note));
            }
            if runtime.canceled_by_owner() {
                runtime.interrupted.store(true, Ordering::SeqCst);
                return Ok(HumanGateWaitResult::Interrupted(Outcome::error(
                    FailureKind::Runtime,
                    "Branch canceled by its fork while waiting for human input",
                    Some(false),
                )));
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
                    runtime.interrupted.store(true, Ordering::SeqCst);
                    return Ok(HumanGateWaitResult::Interrupted(Outcome::error(
                        FailureKind::Runtime,
                        "Run cancel requested while waiting for human input",
                        Some(false),
                    )));
                }
                "pause_requested" | "paused" => {
                    return Ok(HumanGateWaitResult::Interrupted(Outcome::error(
                        FailureKind::Runtime,
                        "Run paused while waiting for human input",
                        Some(false),
                    )));
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
        let _lock = crate::records::lock_run_record(paths)
            .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
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

    /// Runs a fork's declared branches and settles every one before
    /// returning. Each branch ends completed (at the paired join), with a
    /// terminal result (a failure exit), with an execution error, or
    /// canceled by the fork. Records persist as they settle, and each
    /// unsettled branch saves its progress around every invocation, so a
    /// resumed fork never reruns a settled branch or an accepted invocation.
    fn execute_parallel(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let Some(NodeConfig::Parallel {
            branches,
            join,
            max_parallel,
            tolerate_branch_execution_errors: tolerate,
        }) = runtime.node.config.clone()
        else {
            return Ok(Outcome::error(
                FailureKind::Runtime,
                "parallel node has no parallel config",
                Some(false),
            ));
        };
        let state_dir = runtime.run_paths.as_ref().map(|paths| {
            paths
                .logs_dir()
                .join(&runtime.node_id)
                .join("executions")
                .join(format!("{}-{}", runtime.stage_index, runtime.attempt))
                .join("branches")
        });
        let mut records = branches
            .iter()
            .enumerate()
            .map(|(index, _)| {
                state_dir
                    .as_ref()
                    .and_then(|dir| fs::read_to_string(dir.join(format!("{index}.json"))).ok())
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            })
            .collect::<Vec<_>>();
        let stopped = records
            .iter()
            .flatten()
            .any(|record| record_stops_fork(record, tolerate));
        let cancel = Arc::new(AtomicBool::new(stopped));
        self.emit(
            &runtime,
            parallel_started_event(&runtime.run_id, &runtime.node_id, branches.len()),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let pending = (0..branches.len())
            .filter(|index| records[*index].is_none() && !stopped)
            .collect::<Vec<_>>();
        let max_parallel = (max_parallel.unwrap_or(4) as usize).clamp(1, branches.len().max(1));
        let (tx, rx) = mpsc::channel::<(usize, BranchCompletion)>();
        thread::scope(|scope| -> std::result::Result<(), RuntimeNodeError> {
            let mut next = 0_usize;
            let mut in_flight = 0_usize;
            let launch = |index: usize| {
                let tx = tx.clone();
                let runtime = &runtime;
                let entry = branches[index].clone();
                let join = join.clone();
                let cancel = cancel.clone();
                let progress = state_dir
                    .as_ref()
                    .map(|dir| branch_progress_path(dir, index));
                scope.spawn(move || {
                    let _ = self.emit(
                        runtime,
                        parallel_branch_started_event(
                            &runtime.run_id,
                            &runtime.node_id,
                            &entry,
                            index,
                        ),
                    );
                    let record = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.execute_branch(
                            runtime,
                            &entry,
                            index,
                            &join,
                            &cancel,
                            progress.as_deref(),
                        )
                    }))
                    .unwrap_or_else(|payload| {
                        branch_error_record(
                            &entry,
                            index,
                            &entry,
                            FailureKind::Runtime,
                            &panic_payload_message(payload),
                            1,
                        )
                    });
                    let _ = tx.send((index, BranchCompletion { record }));
                });
            };
            while in_flight < max_parallel && next < pending.len() {
                launch(pending[next]);
                next += 1;
                in_flight += 1;
            }
            while in_flight > 0 {
                let (index, completion) = rx.recv().map_err(|_| {
                    RuntimeNodeError::runtime("parallel branch worker disconnected")
                })?;
                in_flight -= 1;
                let record = completion.record;
                // Persist first: a terminal result or fatal error is durable
                // before the fork cancels its siblings.
                if let Some(dir) = state_dir.as_ref() {
                    fs::create_dir_all(dir)
                        .and_then(|_| {
                            fs::write(
                                dir.join(format!("{index}.json")),
                                serde_json::to_vec_pretty(&record).unwrap_or_default(),
                            )
                        })
                        .map_err(|error| {
                            // Settle the siblings before failing the fork.
                            cancel.store(true, Ordering::SeqCst);
                            RuntimeNodeError::terminal(error.to_string())
                        })?;
                }
                if record_stops_fork(&record, tolerate) {
                    cancel.store(true, Ordering::SeqCst);
                }
                let _ = self.emit(
                    &runtime,
                    parallel_branch_completed_event(
                        &runtime.run_id,
                        &runtime.node_id,
                        &branches[index],
                        index,
                        record_status(&record) == "completed",
                    ),
                );
                records[index] = Some(record);
                while !cancel.load(Ordering::SeqCst)
                    && in_flight < max_parallel
                    && next < pending.len()
                {
                    launch(pending[next]);
                    next += 1;
                    in_flight += 1;
                }
            }
            Ok(())
        })?;

        let mut results = records
            .into_iter()
            .enumerate()
            .map(|(index, record)| {
                record.unwrap_or_else(|| {
                    json!({
                        "id": branches[index],
                        "index": index,
                        "status": "canceled",
                        "started": false,
                        "context": {},
                    })
                })
            })
            .collect::<Vec<_>>();
        let sessions = take_branch_sessions(&runtime.context, &mut results);
        let completed = results
            .iter()
            .filter(|record| record_status(record) == "completed")
            .count();
        self.emit(
            &runtime,
            parallel_completed_event(
                &runtime.run_id,
                &runtime.node_id,
                completed,
                results.len() - completed,
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

        let mut context_updates =
            ContextMap::from([("parallel.results".to_string(), json!(results))]);
        if let Some(sessions) = sessions {
            context_updates.insert(crate::context::RUNTIME_SESSIONS_KEY.to_string(), sessions);
        }
        // After settlement: a fatal error wins over a failure exit, which
        // wins over completion; the lowest branch index decides ties.
        if let Some(record) = results
            .iter()
            .find(|record| record_status(record) == "error" && !record_tolerated(record, tolerate))
        {
            let error = &record["error"];
            let kind = serde_json::from_value::<FailureKind>(error["kind"].clone())
                .unwrap_or(FailureKind::Runtime);
            return Ok(Outcome {
                context_updates,
                ..Outcome::error(
                    kind,
                    format!(
                        "branch {} (index {}) failed at {}: {}",
                        object_string(record, "id"),
                        record["index"],
                        object_string(error, "node"),
                        object_string(error, "reason")
                    ),
                    Some(false),
                )
            });
        }
        if let Some(record) = results
            .iter()
            .find(|record| record_status(record) == "terminal_result")
        {
            let terminal = serde_json::from_value::<BranchTerminal>(record["terminal"].clone())
                .map_err(|error| RuntimeNodeError::terminal(error.to_string()))?;
            return Ok(Outcome {
                context_updates,
                notes: format!("branch {} reached {}", terminal.branch, terminal.exit_id),
                terminal: Some(terminal),
                ..Outcome::completed("")
            });
        }
        Ok(Outcome {
            context_updates,
            notes: format!("{completed} of {} branches completed", results.len()),
            ..Outcome::completed("")
        })
    }

    /// One branch invocation with the root's rules: the same retry policy,
    /// exact dispatch and visit limits (counted per branch). It stops at its
    /// fork's join or at a failure exit and never executes either.
    fn execute_branch(
        &self,
        runtime: &HandlerRuntime,
        entry: &str,
        index: usize,
        join: &str,
        cancel: &Arc<AtomicBool>,
        progress_path: Option<&Path>,
    ) -> Value {
        let flow = &runtime.flow;
        let saved = progress_path.and_then(read_branch_progress);
        let mut values = crate::flow_runtime::flow_context_seed(flow);
        values.extend(runtime.context.clone());
        let mut context = match AttractorContext::from_map(values)
            .map_err(crate::error::RuntimeStorageError::from)
            .and_then(|mut context| {
                seed_builtin_context(&mut context, entry)?;
                clear_runtime_retry_context(&mut context)?;
                Ok(context)
            }) {
            Ok(context) => context,
            Err(error) => {
                return branch_error_record(
                    entry,
                    index,
                    entry,
                    FailureKind::Runtime,
                    &error.to_string(),
                    0,
                )
            }
        };
        let mut current = entry.to_string();
        let mut visits = BTreeMap::from([(current.clone(), 1u64)]);
        let mut retries = BTreeMap::<String, u64>::new();
        let mut trace = vec![current.clone()];
        let mut decisions = Vec::<Value>::new();
        let mut steps = 0_u64;
        let mut invocations = 0_u64;
        let mut last: Option<(String, String, String)> = None;
        // A resumed fork continues each unsettled branch where it stopped:
        // the same context, position, budgets and, for an invocation that
        // completed before the interruption, its accepted result.
        let mut accepted = None;
        if let Some(saved) = saved {
            match AttractorContext::from_map(saved.context) {
                Ok(restored) => context = restored,
                Err(error) => {
                    return branch_error_record(
                        entry,
                        index,
                        &saved.current,
                        FailureKind::Runtime,
                        &error.to_string(),
                        0,
                    )
                }
            }
            current = saved.current;
            visits = saved.visits;
            retries = saved.retries;
            trace = saved.trace;
            decisions = saved.decisions;
            steps = saved.steps;
            invocations = saved.invocations;
            last = saved.last;
            accepted = saved.accepted;
        }
        macro_rules! save_progress {
            ($started:expr, $accepted:expr) => {{
                let progress = BranchProgress {
                    context: context.snapshot(),
                    current: current.clone(),
                    visits: visits.clone(),
                    retries: retries.clone(),
                    trace: trace.clone(),
                    decisions: decisions.clone(),
                    steps,
                    invocations,
                    last: last.clone(),
                    started: $started,
                    accepted: $accepted,
                };
                match progress_path.map(|path| write_branch_progress(path, &progress)) {
                    Some(Err(error)) => Err(error),
                    _ => Ok(()),
                }
            }};
        }
        macro_rules! record {
            ($status:expr, $extra:expr $(,)?) => {{
                let mut record = json!({
                    "id": entry,
                    "index": index,
                    "status": $status,
                    "started": true,
                    "context": context.snapshot(),
                    "route_trace": trace,
                    "decisions": decisions,
                    "visits": visits,
                    "fork_attempt": runtime.attempt,
                    "notes": last.as_ref().map(|(_, _, notes): &(String, String, String)| notes.clone()).unwrap_or_default(),
                });
                if let (Some(record), Value::Object(extra)) = (record.as_object_mut(), $extra) {
                    record.extend(extra);
                }
                record
            }};
        }
        loop {
            if current == join {
                return record!("completed", json!({}));
            }
            if is_exit_node(flow, &current) {
                // Validation forbids success exits in branches; a branch can
                // never end its owner successfully before the join.
                if flow
                    .exit_result(&current)
                    .is_some_and(|result| result.outcome == attractor_core::ExitOutcome::Success)
                {
                    return record!(
                        "error",
                        error_value(
                            &current,
                            FailureKind::Routing,
                            &format!("branch reached success exit {current} before its join"),
                            0
                        )
                    );
                }
                let Some(result) = flow.exit_result(&current).cloned() else {
                    return record!(
                        "error",
                        error_value(&current, FailureKind::Routing, "exit declares no result", 0)
                    );
                };
                let (node, decision, notes) = last.clone().unwrap_or_default();
                let terminal = BranchTerminal {
                    exit_id: current.clone(),
                    result,
                    branch: entry.to_string(),
                    branch_index: index,
                    node,
                    decision,
                    notes,
                };
                return record!("terminal_result", json!({ "terminal": terminal }));
            }
            if cancel.load(Ordering::SeqCst) || run_cancel_requested(runtime) {
                return record!("canceled", json!({"canceled_at": current}));
            }
            let Some(node) = flow.nodes.get(&current).cloned() else {
                return record!(
                    "error",
                    error_value(
                        &current,
                        FailureKind::Routing,
                        &format!("unknown node {current}"),
                        0
                    )
                );
            };
            // Every branch invocation has its own execution directory: the
            // fork invocation's stage, then the branch's invocation number
            // (under the fork's attempt). An interrupted invocation keeps
            // its number on resume.
            // ponytail: one million invocations per branch per fork attempt.
            let attempt = runtime.attempt * BRANCH_INVOCATIONS_PER_ATTEMPT + invocations;
            let _ = context.set("current_node", json!(current.clone()));
            let outgoing_edges = outgoing_routing_edges(flow, &current).unwrap_or_default();
            let node_attrs = crate::flow_runtime::node_attrs_for_handler(&current, &node);
            let mut request_context = context.snapshot();
            mark_unavailable_routes(&mut request_context, flow, &outgoing_edges, &visits);
            let request = NodeExecutionRequest {
                node_id: current.clone(),
                stage_index: runtime.stage_index,
                attempt,
                context: request_context,
                prompt: crate::flow_runtime::node_prompt(&node),
                node: node.clone(),
                node_attrs: node_attrs.clone(),
                flow: flow.clone(),
                outgoing_edges,
                run_paths: runtime.run_paths.clone(),
                run_workdir: runtime.run_workdir.clone(),
                run_id: runtime.run_id.clone(),
                fallback_model: runtime.fallback_model.clone(),
                fallback_provider: runtime.fallback_provider.clone(),
                fallback_profile: runtime.fallback_profile.clone(),
                fallback_reasoning_effort: runtime.fallback_reasoning_effort.clone(),
            };
            let _ = self.emit(
                runtime,
                parallel_branch_node_started_event(
                    &runtime.run_id,
                    &runtime.node_id,
                    entry,
                    index,
                    &current,
                ),
            );
            // An invocation accepted before an interruption is never run
            // again; its context updates are already in the saved context.
            let mut interrupted = Arc::new(AtomicBool::new(false));
            let (raw, already_applied) = match accepted.take() {
                Some(outcome) => (outcome, true),
                None => {
                    if let Err(error) = save_progress!(Some(current.clone()), None) {
                        return record!(
                            "error",
                            error_value(&current, FailureKind::Runtime, &error, 0)
                        );
                    }
                    let mut node_runtime = HandlerRuntime::from_request(request);
                    node_runtime.cancel = Some(cancel.clone());
                    interrupted = node_runtime.interrupted.clone();
                    let raw = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.execute_runtime(node_runtime)
                    })) {
                        Ok(Ok(outcome)) => outcome,
                        Ok(Err(error)) => outcome_from_node_error(error),
                        Err(payload) => outcome_from_node_error(RuntimeNodeError::terminal(
                            panic_payload_message(payload),
                        )),
                    };
                    invocations += 1;
                    (raw, false)
                }
            };
            let _ = self.emit(
                runtime,
                parallel_branch_node_completed_event(
                    &runtime.run_id,
                    &runtime.node_id,
                    entry,
                    index,
                    &current,
                    raw.status.to_string(),
                ),
            );
            let outcome = if already_applied {
                raw
            } else {
                match crate::context::apply_outcome_context_updates(
                    &current,
                    &node_attrs,
                    &mut context,
                    &raw,
                ) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        outcome_from_node_error(RuntimeNodeError::terminal(error.to_string()))
                    }
                }
            };
            if outcome.is_error() {
                // An invocation the fork's or the user's cancellation actually
                // interrupted is canceled, with the interruption as evidence.
                // Any other error, of any kind, is the branch's own and keeps
                // its kind, reason and precedence.
                let kind = outcome.error_kind().unwrap_or(FailureKind::Runtime);
                if interrupted.load(Ordering::SeqCst) {
                    return record!(
                        "canceled",
                        json!({
                            "canceled_at": current,
                            "interruption": stage_failure_reason(&outcome),
                        })
                    );
                }
                let retries_so_far = retries.get(&current).copied().unwrap_or(0);
                let policy = retry_policy_for_outcome(flow, &current, &outcome);
                if should_retry_attempt(&outcome, retries_so_far, &policy) {
                    retries.insert(current.clone(), retries_so_far + 1);
                    if let Err(error) = save_progress!(None, None) {
                        return record!(
                            "error",
                            error_value(&current, FailureKind::Runtime, &error, 0)
                        );
                    }
                    continue;
                }
                return record!(
                    "error",
                    error_value(
                        &current,
                        kind,
                        &stage_failure_reason(&outcome),
                        retries_so_far + 1
                    ),
                );
            }
            if !already_applied {
                // The accepted result is durable before its transition.
                let stored = Outcome {
                    context_updates: ContextMap::new(),
                    raw_response_text: String::new(),
                    ..outcome.clone()
                };
                if let Err(error) = save_progress!(Some(current.clone()), Some(stored)) {
                    return record!(
                        "error",
                        error_value(&current, FailureKind::Runtime, &error, 0)
                    );
                }
            }
            retries.remove(&current);
            steps += 1;
            let transition = match select_transition(flow, &current, &outcome, &visits) {
                Ok(transition) => transition,
                Err(reason) => {
                    return record!(
                        "error",
                        error_value(&current, FailureKind::Routing, &reason, 1)
                    )
                }
            };
            decisions.push(json!({
                "node": current,
                "decision": transition.decision,
                "target": transition.target,
            }));
            last = Some((
                current.clone(),
                transition.decision.clone(),
                outcome.notes.clone(),
            ));
            *visits.entry(transition.target.clone()).or_default() += 1;
            trace.push(transition.target.clone());
            current = transition.target;
            if let Err(error) = save_progress!(None, None) {
                return record!(
                    "error",
                    error_value(&current, FailureKind::Runtime, &error, 0)
                );
            }
        }
    }

    /// A join is a structural barrier: the fork already settled every
    /// branch, so it continues with `next`.
    fn execute_fan_in(
        &self,
        runtime: HandlerRuntime,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        let results = runtime
            .context
            .get("parallel.results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let completed = results
            .iter()
            .filter(|record| record_status(record) == "completed")
            .count();
        Ok(Outcome {
            notes: format!(
                "{completed} of {} branches completed; see parallel.results",
                results.len()
            ),
            ..Outcome::completed(attractor_core::NEXT_DECISION)
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
            Ok(Err(error)) => Ok(outcome_from_node_error(error)),
            Err(payload) => Ok(Outcome::error(
                FailureKind::Runtime,
                panic_payload_message(payload),
                Some(false),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    key: String,
    label: String,
    decision: String,
}

impl Choice {
    /// The published option: its value is the decision ID.
    fn option(&self) -> HumanOption {
        HumanOption {
            label: self.label.clone(),
            value: self.decision.clone(),
            key: self.key.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
    timed_out: bool,
    interrupted: bool,
}

/// The decision a finished tool command selects through its node's one
/// adapter, with `context.tool.*` and any `output_map` values applied
/// together. Unmapped exits, malformed JSON and undeclared decisions are
/// execution errors carrying the exit code and stderr.
fn tool_decision_outcome(runtime: &HandlerRuntime, output: &ShellOutput) -> Outcome {
    let Some(NodeConfig::Tool {
        exit_codes,
        decision_field,
        ..
    }) = runtime.node.config.as_ref()
    else {
        return Outcome::error(
            FailureKind::Runtime,
            "tool node has no tool config",
            Some(false),
        );
    };
    let mut context_updates = tool_context_updates(output.stdout.trim(), output.exit_code);
    let evidence = |message: String| {
        let stderr = output.stderr.trim();
        if stderr.is_empty() {
            message
        } else {
            format!("{message}; stderr: {stderr}")
        }
    };
    let decision = match decision_field.as_deref() {
        Some(field) if output.exit_code != 0 => Err(evidence(format!(
            "tool exited with code {} instead of 0 with a JSON {field} decision",
            output.exit_code
        ))),
        Some(field) => match serde_json::from_str::<Value>(output.stdout.trim()) {
            Ok(Value::Object(object)) => match object.get(field) {
                Some(Value::String(decision)) => Ok(decision.clone()),
                Some(_) => Err(evidence(format!("tool JSON field {field} is not a string"))),
                None => Err(evidence(format!("tool JSON output has no {field} field"))),
            },
            _ => Err(evidence(format!(
                "tool stdout is not one JSON object with a {field} decision"
            ))),
        },
        None => exit_codes
            .iter()
            .find(|(code, _)| code.trim().parse::<i32>().ok() == Some(output.exit_code))
            .map(|(_, decision)| decision.clone())
            .ok_or_else(|| {
                evidence(format!(
                    "tool exited with unmapped code {}",
                    output.exit_code
                ))
            }),
    };
    let decision = match decision {
        Ok(decision) if runtime.node.routes.contains_key(&decision) => decision,
        Ok(decision) => {
            return Outcome {
                context_updates,
                ..Outcome::error(
                    FailureKind::Contract,
                    evidence(format!("tool returned undeclared decision {decision:?}")),
                    Some(false),
                )
            }
        }
        Err(reason) => {
            return Outcome {
                context_updates,
                ..Outcome::error(FailureKind::Runtime, reason, None)
            }
        }
    };
    match resolve_tool_output_updates(runtime, &output.stdout) {
        Ok(output_updates) => {
            context_updates.extend(output_updates);
            Outcome {
                context_updates,
                notes: output.stdout.trim().to_string(),
                ..Outcome::completed(decision)
            }
        }
        Err(reason) => Outcome {
            context_updates,
            ..Outcome::error(FailureKind::Contract, reason, Some(false))
        },
    }
}

/// A switch reads its key once and compares exact scalars. A missing key
/// or an unsupported value is an execution error, never `otherwise`.
fn execute_switch(runtime: &HandlerRuntime) -> Outcome {
    let Some(NodeConfig::Switch {
        on,
        cases,
        otherwise,
    }) = runtime.node.config.as_ref()
    else {
        return Outcome::error(
            FailureKind::Runtime,
            "switch node has no switch config",
            Some(false),
        );
    };
    let Some(value) = runtime.context.get(on) else {
        return Outcome::error(
            FailureKind::Contract,
            format!("switch key {on} is missing from the context"),
            Some(false),
        );
    };
    match attractor_core::switch_decision(cases, otherwise, value) {
        Ok(decision) => Outcome {
            notes: format!("{on} = {value}"),
            ..Outcome::completed(decision)
        },
        Err(reason) => Outcome::error(
            FailureKind::Contract,
            format!("switch key {on}: {reason}"),
            Some(false),
        ),
    }
}

fn tool_context_updates(output: &str, exit_code: i32) -> ContextMap {
    ContextMap::from([
        ("context.tool.output".to_string(), json!(output)),
        ("context.tool.exit_code".to_string(), json!(exit_code)),
    ])
}

fn tool_attr_string_map(
    runtime: &HandlerRuntime,
    attr: &str,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let Some(raw) = attr_text(&runtime.node_attrs, attr) else {
        return Ok(BTreeMap::new());
    };
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str::<BTreeMap<String, String>>(&raw)
        .map_err(|error| format!("{attr} must be an object of string pairs: {error}"))
}

fn resolve_tool_env_map(
    runtime: &HandlerRuntime,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let bindings = tool_attr_string_map(runtime, "tool.env_map")?;
    let mut env = BTreeMap::new();
    for (name, context_key) in bindings {
        if !is_valid_env_var_name(&name) {
            return Err(format!(
                "tool.env_map name '{name}' is not a valid environment variable name"
            ));
        }
        let value = crate::manager_loop::context_path_value(&runtime.context, &context_key)
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                format!("tool.env_map key {context_key} did not resolve to a context value")
            })?;
        let text = match value {
            Value::String(text) => text,
            Value::Bool(_) | Value::Number(_) => value.to_string(),
            other => serde_json::to_string(&other).map_err(|error| {
                format!("tool.env_map key {context_key} could not be serialized: {error}")
            })?,
        };
        env.insert(name, text);
    }
    Ok(env)
}

fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn resolve_tool_output_updates(
    runtime: &HandlerRuntime,
    stdout: &str,
) -> std::result::Result<ContextMap, String> {
    let bindings = tool_attr_string_map(runtime, "tool.output_map")?;
    let mut updates = ContextMap::new();
    if bindings.is_empty() {
        return Ok(updates);
    }
    let parsed: Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        format!("tool.output_map requires the tool to print a JSON object: {error}")
    })?;
    for (context_key, source_path) in bindings {
        let mut current = &parsed;
        for segment in source_path.split('.') {
            let segment = segment.trim();
            if segment.is_empty() {
                return Err(format!(
                    "tool.output_map path '{source_path}' contains an empty segment"
                ));
            }
            current = current.get(segment).ok_or_else(|| {
                format!("tool.output_map path '{source_path}' not found in tool JSON output")
            })?;
        }
        updates.insert(context_key, current.clone());
    }
    Ok(updates)
}

/// Presents what the reviewer is deciding on: the user-space context the
/// immediately preceding node produced, recorded by the runtime for every
/// node. Generic — no flow declarations involved.
fn human_gate_details(runtime: &HandlerRuntime) -> Option<String> {
    let updates = runtime
        .context
        .get(crate::context::RUNTIME_LAST_NODE_UPDATES_KEY)
        .and_then(Value::as_object)?;
    let sections = updates
        .iter()
        .filter_map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Null => return None,
                other => serde_json::to_string_pretty(other).ok()?,
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let heading = key.strip_prefix("context.").unwrap_or(key);
            Some(format!("### {heading}\n\n{text}"))
        })
        .collect::<Vec<_>>();
    (!sections.is_empty()).then(|| sections.join("\n\n"))
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
    run_shell_command(command, cwd, None, env, Some(payload.to_string()), None).unwrap_or_else(
        |error| ShellOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: error.to_string(),
            timed_out: false,
            interrupted: false,
        },
    )
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

/// Runs `command`, draining stdout and stderr concurrently so a chatty
/// command never blocks on a full pipe, and kills it at `timeout` or when
/// `stop` answers true.
fn run_shell_command(
    command: &str,
    cwd: &Path,
    timeout: Option<Duration>,
    env: BTreeMap<String, String>,
    stdin_text: Option<String>,
    mut stop: Option<&mut dyn FnMut() -> bool>,
) -> std::io::Result<ShellOutput> {
    let mut command_line = Command::new("sh");
    // Its own process group, so a timeout or cancellation stops every
    // descendant, not just the shell.
    spark_agent_adapter::local_environment::configure_process_group(&mut command_line);
    let mut child = command_line
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

    if timeout.is_none() && stop.is_none() {
        let output = child.wait_with_output()?;
        return Ok(shell_output_from_process(output, false, false));
    }

    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = drain(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let started_at = Instant::now();
    let (status, timed_out, interrupted) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false, false);
        }
        let timed_out = timeout.is_some_and(|timeout| started_at.elapsed() >= timeout);
        let interrupted = !timed_out && stop.as_mut().is_some_and(|stop| stop());
        if timed_out || interrupted {
            spark_agent_adapter::local_environment::terminate_process_group(
                child.id(),
                Some(&mut child),
            );
            let _ = child.kill();
            break (child.wait()?, timed_out, interrupted);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    };
    Ok(shell_output_from_process(output, timed_out, interrupted))
}

fn shell_output_from_process(
    output: std::process::Output,
    timed_out: bool,
    interrupted: bool,
) -> ShellOutput {
    ShellOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        timed_out,
        interrupted,
    }
}

const HUMAN_GATE_POLL_INTERVAL_MS: u64 = 250;

enum HumanGateWaitResult {
    Answered(Choice, String),
    Interrupted(Outcome),
}

enum GateRunStatus {
    Waiting,
    Running,
}

fn events_length(paths: &RunRootPaths) -> u64 {
    std::fs::metadata(paths.events_jsonl())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// The first answer to `question_id` in `events` that names a declared choice.
fn valid_gate_answer<'a>(
    events: impl Iterator<Item = &'a attractor_core::RawRuntimeEvent>,
    question_id: &str,
    choices: &[Choice],
) -> Option<(Choice, String)> {
    let mut events = events;
    std::iter::from_fn(|| gate_answer_in(&mut events, question_id)).find_map(|(answer, note)| {
        select_choice(&HumanAnswer::selected(answer), choices).map(|choice| (choice, note))
    })
}

/// Latest-first scan for a completed answer to `question_id`; callers pass
/// events in the priority order they want (full history newest-first, or an
/// appended batch in arrival order).
fn gate_answer_in<'a>(
    events: impl Iterator<Item = &'a attractor_core::RawRuntimeEvent>,
    question_id: &str,
) -> Option<(String, String)> {
    let mut events = events;
    events.find_map(|event| {
        if event.event_type != "InterviewCompleted" {
            return None;
        }
        if event.payload.get("question_id").and_then(Value::as_str) != Some(question_id) {
            return None;
        }
        let answer = event
            .payload
            .get("answer")
            .and_then(Value::as_str)
            .map(str::to_string)?;
        let note = event
            .payload
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        Some((answer, note))
    })
}

/// The declared decision an answer names. Only exact decision IDs count:
/// labels and accelerator keys are display, translated to the decision by
/// the UI before submission, so a label can never capture another decision.
fn select_choice(answer: &HumanAnswer, choices: &[Choice]) -> Option<Choice> {
    let mut tokens = answer
        .selected_values
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
        .collect::<Vec<_>>();
    if let Some(option) = &answer.selected_option {
        tokens.extend(trimmed_nonempty(&option.value));
    }
    tokens.extend(trimmed_nonempty(&answer.text));
    tokens.extend(trimmed_nonempty(&answer.value));
    tokens
        .iter()
        .find_map(|token| choices.iter().find(|choice| choice.decision == *token))
        .cloned()
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
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

/// Removes the runtime's session handles from each branch payload, which
/// agents can read, and returns the parent's sessions with every entry a
/// branch started or continued, or `None` when no branch changed one.
// ponytail: branches updating the same session race; last completed wins.
fn take_branch_sessions(parent: &ContextMap, results: &mut [Value]) -> Option<Value> {
    let key = crate::context::RUNTIME_SESSIONS_KEY;
    let before = parent.get(key).cloned().unwrap_or_else(|| json!({}));
    let mut merged = before.clone();
    for result in results.iter_mut() {
        let Some(branch) = result
            .get_mut("context")
            .and_then(Value::as_object_mut)
            .and_then(|context| context.remove(key))
        else {
            continue;
        };
        for (section, entries) in branch.as_object().into_iter().flatten() {
            for (name, handle) in entries.as_object().into_iter().flatten() {
                if before.get(section).and_then(|s| s.get(name)) != Some(handle) {
                    merged[section][name] = handle.clone();
                }
            }
        }
    }
    (merged != before).then_some(merged)
}

const BRANCH_INVOCATIONS_PER_ATTEMPT: u64 = 1_000_000;

/// An unsettled branch's position, saved in its fork's execution directory
/// before and after every invocation and transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BranchProgress {
    context: ContextMap,
    current: String,
    visits: BTreeMap<String, u64>,
    retries: BTreeMap<String, u64>,
    trace: Vec<String>,
    decisions: Vec<Value>,
    steps: u64,
    /// Invocations this branch finished, retries included; numbers the
    /// next invocation's execution identity.
    #[serde(default)]
    invocations: u64,
    last: Option<(String, String, String)>,
    /// The node whose invocation began and has not been accepted yet.
    #[serde(default)]
    started: Option<String>,
    /// That node's accepted result, saved before its transition.
    #[serde(default)]
    accepted: Option<Outcome>,
}

fn branch_progress_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("{index}.progress.json"))
}

fn read_branch_progress(path: &Path) -> Option<BranchProgress> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_branch_progress(
    path: &Path,
    progress: &BranchProgress,
) -> std::result::Result<(), String> {
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, serde_json::to_vec(progress).unwrap_or_default())?;
        fs::rename(&temp, path)
    };
    write().map_err(|error| format!("could not save branch progress: {error}"))
}

/// The branch node of the fork at `fork_id` (stage/attempt) whose invocation
/// was interrupted before it was accepted and whose `recovery_policy` is
/// `pause`: resuming would rerun it, so recovery needs an explicit decision,
/// as for a root node.
pub fn fork_branch_requiring_recovery_decision(
    paths: &RunRootPaths,
    flow: &FlowDefinition,
    fork_id: &str,
    stage_index: u64,
    attempt: u64,
) -> Option<String> {
    let Some(NodeConfig::Parallel { branches, .. }) = flow.nodes.get(fork_id)?.config.as_ref()
    else {
        return None;
    };
    let dir = paths
        .logs_dir()
        .join(fork_id)
        .join("executions")
        .join(format!("{stage_index}-{attempt}"))
        .join("branches");
    (0..branches.len()).find_map(|index| {
        if dir.join(format!("{index}.json")).exists() {
            return None;
        }
        let progress = read_branch_progress(&branch_progress_path(&dir, index))?;
        let node = progress.started.filter(|_| progress.accepted.is_none())?;
        flow.nodes
            .get(&node)?
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.recovery_policy == attractor_core::RecoveryPolicy::Pause)
            .then_some(node)
    })
}

/// Whether the user asked to cancel the run this branch belongs to.
fn run_cancel_requested(runtime: &HandlerRuntime) -> bool {
    runtime
        .run_paths
        .as_ref()
        .is_some_and(run_record_cancel_requested)
}

fn run_record_cancel_requested(paths: &RunRootPaths) -> bool {
    crate::records::read_run_record(paths)
        .ok()
        .flatten()
        .is_some_and(|record| {
            matches!(
                crate::records::normalize_run_status(&record.status).as_str(),
                "cancel_requested" | "canceled"
            )
        })
}

fn record_status(record: &Value) -> &str {
    record
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// An execution error a tolerant fork keeps.
fn record_tolerated(record: &Value, tolerate: bool) -> bool {
    tolerate && record["error"]["tolerable"].as_bool() == Some(true)
}

/// A settled branch that stops the fork launching more work.
fn record_stops_fork(record: &Value, tolerate: bool) -> bool {
    match record_status(record) {
        "terminal_result" => true,
        "error" => !record_tolerated(record, tolerate),
        _ => false,
    }
}

fn error_value(node: &str, kind: FailureKind, reason: &str, attempts: u64) -> Value {
    json!({
        "error": {
            "node": node,
            "kind": kind,
            "tolerable": kind.is_tolerable(),
            "reason": reason,
            "attempts": attempts,
        }
    })
}

fn branch_error_record(
    entry: &str,
    index: usize,
    node: &str,
    kind: FailureKind,
    reason: &str,
    attempts: u64,
) -> Value {
    let mut record = json!({
        "id": entry,
        "index": index,
        "status": "error",
        "started": true,
        "context": {},
    });
    record["error"] = error_value(node, kind, reason, attempts)["error"].clone();
    record
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

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn codergen_event_duplicates_share_externalized_tool_output() {
        let temp = tempfile::tempdir().expect("tempdir");
        let activity = spark_storage::ActivityRepository::new(temp.path());
        let full_output = "large output".repeat(2_000);
        let tool_call = json!({"id": "call-1", "output": full_output.clone()});
        let mut event = CodergenEvent::new(
            "codex_app_server_session_event",
            BTreeMap::from([
                ("tool_event".to_string(), tool_call.clone()),
                (
                    "turn_stream_event".to_string(),
                    json!({"kind": "tool_call_completed", "tool_call": tool_call}),
                ),
            ]),
        );

        externalize_codergen_tool_output(&activity, &mut event).expect("externalize");

        assert_eq!(
            event.payload["tool_event"],
            event.payload["turn_stream_event"]["tool_call"]
        );
        assert_eq!(event.payload["tool_event"]["output_truncated"], true);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("tool-output/call-1.txt")).expect("artifact"),
            full_output
        );
    }
}
