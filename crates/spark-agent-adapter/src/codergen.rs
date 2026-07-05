use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use attractor_core::{
    attr_bool, attr_string, attr_text, dot_value_text, node_has_explicit_attr,
    resolve_context_read_contract, resolve_context_write_contract, ContextMap,
    ContextWriteContract, DotAttribute, DotGraph, DotNode, DotValue, FailureKind, Outcome,
    OutcomeStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::debug::{
    codex_jsonrpc_trace_enabled, CODEX_JSONRPC_TRACE_FILE_NAME,
    CODEX_JSONRPC_TRACE_PATH_METADATA_KEY,
};
use spark_storage::{write_json_atomic, write_text_atomic, JsonWriteOptions};
use thiserror::Error;
use unified_llm_adapter::{
    resolve_effective_llm_model, resolve_effective_llm_profile, resolve_effective_llm_provider,
    resolve_effective_reasoning_effort, LlmResolutionInputs, Usage,
};

use crate::session::SessionSteeringHandle;
use crate::status_envelope::{
    build_contract_repair_prompt, build_status_envelope_prompt_appendix,
    coerce_structured_text_outcome, contract_failure_outcome, validate_write_contract_violation,
    StructuredContractViolation, StructuredTextOutcome,
};

pub const RUNTIME_CONTEXT_CARRYOVER_KEY: &str = "_attractor.runtime.context_carryover";
pub const DECLARED_CONTEXT_MISSING_SENTINEL: &str = "<missing>";
pub const STATUS_ENVELOPE_RESPONSE_CONTRACT: &str = "status_envelope";
pub const DEFAULT_CONTRACT_REPAIR_ATTEMPTS: u32 = 1;

#[derive(Debug, Error)]
pub enum CodergenError {
    #[error("codergen backend failed: {0}")]
    Backend(String),
    #[error("codergen artifact write failed: {0}")]
    Artifact(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodergenRequest {
    pub node_id: String,
    pub node: DotNode,
    pub graph: DotGraph,
    #[serde(default)]
    pub context: ContextMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodergenExecutionMode {
    #[default]
    #[serde(alias = "text", alias = "completion")]
    TextOnly,
    #[serde(alias = "agent_required", alias = "session")]
    Agent,
}

impl CodergenExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextOnly => "text_only",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodergenRuntimeMode {
    #[serde(default)]
    pub mode: CodergenExecutionMode,
    #[serde(default)]
    pub requires_tools: bool,
    #[serde(default)]
    pub requires_steering: bool,
    #[serde(default)]
    pub requires_child_intervention: bool,
    #[serde(default)]
    pub requires_session_events: bool,
}

impl CodergenRuntimeMode {
    pub fn text_only() -> Self {
        Self::default()
    }

    pub fn agent() -> Self {
        Self {
            mode: CodergenExecutionMode::Agent,
            requires_tools: true,
            requires_steering: true,
            requires_child_intervention: true,
            requires_session_events: true,
        }
    }

    pub fn requires_agent(&self) -> bool {
        self.mode == CodergenExecutionMode::Agent
            || self.requires_tools
            || self.requires_steering
            || self.requires_child_intervention
            || self.requires_session_events
    }

    pub fn is_text_only(&self) -> bool {
        !self.requires_agent()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodergenBackendRequest {
    pub node_id: String,
    pub prompt: String,
    #[serde(default)]
    pub context: ContextMap,
    #[serde(default)]
    pub response_contract: String,
    #[serde(default)]
    pub contract_repair_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub write_contract: ContextWriteContract,
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "CodergenRuntimeMode::is_text_only")]
    pub runtime_mode: CodergenRuntimeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodergenChildInterventionRequest {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodergenChildInterventionResult {
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

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCodergenSession {
    pub node_id: String,
    pub child_run_id: Option<String>,
    pub root_run_id: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub llm_profile: Option<String>,
    pub reasoning_effort: Option<String>,
    pub project_path: Option<PathBuf>,
    pub metadata: BTreeMap<String, Value>,
    pub steering: SessionSteeringHandle,
}

#[derive(Debug, Clone, Default)]
pub struct CodergenSessionInterventionBroker {
    inner: Arc<Mutex<CodergenSessionInterventionState>>,
}

#[derive(Debug, Clone)]
struct ActiveCodergenSessionEntry {
    generation: u64,
    session: ActiveCodergenSession,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ActiveCodergenSessionKey {
    child_run_id: String,
    root_run_id: Option<String>,
    target_node_id: String,
}

impl ActiveCodergenSessionKey {
    fn from_session(session: &ActiveCodergenSession) -> Option<Self> {
        Some(Self {
            child_run_id: normalized_optional(session.child_run_id.as_deref())?,
            root_run_id: normalized_optional(session.root_run_id.as_deref()),
            target_node_id: normalized_optional(Some(session.node_id.as_str()))?,
        })
    }
}

#[derive(Debug, Default)]
struct CodergenSessionInterventionState {
    generation: u64,
    active: BTreeMap<ActiveCodergenSessionKey, ActiveCodergenSessionEntry>,
    unkeyed_active: BTreeMap<u64, ActiveCodergenSessionEntry>,
}

#[derive(Debug)]
pub struct ActiveCodergenSessionGuard {
    broker: CodergenSessionInterventionBroker,
    generation: u64,
    key: Option<ActiveCodergenSessionKey>,
}

impl Drop for ActiveCodergenSessionGuard {
    fn drop(&mut self) {
        let active = {
            let mut state = self
                .broker
                .inner
                .lock()
                .expect("codergen intervention broker lock");
            if let Some(key) = self.key.as_ref() {
                if state
                    .active
                    .get(key)
                    .is_some_and(|entry| entry.generation == self.generation)
                {
                    state.active.remove(key)
                } else {
                    None
                }
            } else {
                state.unkeyed_active.remove(&self.generation)
            }
        };
        if let Some(active) = active {
            active.session.steering.close();
        }
    }
}

impl CodergenSessionInterventionBroker {
    pub fn register(&self, session: ActiveCodergenSession) -> ActiveCodergenSessionGuard {
        let key = ActiveCodergenSessionKey::from_session(&session);
        let mut state = self
            .inner
            .lock()
            .expect("codergen intervention broker lock");
        state.generation = state.generation.saturating_add(1);
        let generation = state.generation;
        let entry = ActiveCodergenSessionEntry {
            generation,
            session,
        };
        let replaced = if let Some(key) = key.clone() {
            state.active.insert(key, entry)
        } else {
            state.unkeyed_active.insert(generation, entry)
        };
        drop(state);
        if let Some(replaced) = replaced {
            replaced.session.steering.close();
        }
        ActiveCodergenSessionGuard {
            broker: self.clone(),
            generation,
            key,
        }
    }

    pub fn request_child_intervention(
        &self,
        request: CodergenChildInterventionRequest,
    ) -> CodergenChildInterventionResult {
        let (active_sessions, has_unkeyed_active) = {
            let state = self
                .inner
                .lock()
                .expect("codergen intervention broker lock");
            (
                state
                    .active
                    .values()
                    .map(|entry| entry.session.clone())
                    .collect::<Vec<_>>(),
                !state.unkeyed_active.is_empty(),
            )
        };
        if active_sessions.is_empty() {
            if has_unkeyed_active {
                return rejected_intervention(
                    &request,
                    "backend_steering_unsupported",
                    "rust_session",
                    "No active Rust codergen session has an intervention child run identity.",
                );
            }
            return rejected_intervention(
                &request,
                "backend_steering_unsupported",
                "rust_session",
                "No active Rust codergen session is available for intervention.",
            );
        }

        let request_child_run_id = request.child_run_id.trim();
        let child_matches = active_sessions
            .into_iter()
            .filter(|active| {
                normalized_optional(active.child_run_id.as_deref()).as_deref()
                    == Some(request_child_run_id)
            })
            .collect::<Vec<_>>();
        if child_matches.is_empty() {
            return rejected_intervention(
                &request,
                "backend_steering_unsupported",
                "rust_session",
                "No active Rust codergen session matches the intervention child run.",
            );
        }

        let request_root_run_id = normalized_optional(Some(request.root_run_id.as_str()));
        let (root_matches, rootless_matches): (Vec<_>, Vec<_>) =
            child_matches.into_iter().partition(|active| {
                normalized_optional(active.root_run_id.as_deref()).as_deref()
                    == request_root_run_id.as_deref()
            });
        let root_matches = if request_root_run_id.is_some() && root_matches.is_empty() {
            rootless_matches
                .into_iter()
                .filter(|active| normalized_optional(active.root_run_id.as_deref()).is_none())
                .collect::<Vec<_>>()
        } else {
            root_matches
        };
        if root_matches.is_empty() {
            return rejected_intervention(
                &request,
                "backend_steering_unsupported",
                "rust_session",
                "No active Rust codergen session matches the intervention root run.",
            );
        }

        let active = if let Some(target_node_id) =
            normalized_optional(request.target_node_id.as_deref())
        {
            let target_matches = root_matches
                .into_iter()
                .filter(|active| active.node_id == target_node_id)
                .collect::<Vec<_>>();
            match target_matches.into_iter().next() {
                Some(active) => active,
                None => {
                    return rejected_intervention(
                        &request,
                        "backend_steering_unsupported",
                        "rust_session",
                        "No active Rust codergen session matches the intervention target node.",
                    );
                }
            }
        } else {
            let mut matches = root_matches.into_iter();
            let Some(active) = matches.next() else {
                return rejected_intervention(
                    &request,
                    "backend_steering_unsupported",
                    "rust_session",
                    "No active Rust codergen session matches the intervention target node.",
                );
            };
            if matches.next().is_some() {
                return rejected_intervention(
                    &request,
                    "backend_steering_unsupported",
                    "rust_session",
                    "Multiple active Rust codergen sessions match the intervention child run and root run; target node is required.",
                );
            }
            active
        };

        let payload = intervention_steering_payload(&request, &active);
        if !active
            .steering
            .queue_steering_with_metadata(request.message.clone(), payload)
        {
            return rejected_intervention(
                &request,
                "backend_steering_unsupported",
                "rust_session",
                "Active Rust codergen session is no longer accepting steering.",
            );
        }
        let is_codex_app_server = active.provider.trim().eq_ignore_ascii_case("codex")
            && normalized_optional(active.llm_profile.as_deref()).is_none();
        CodergenChildInterventionResult {
            run_id: request.child_run_id,
            status: "delivered".to_string(),
            delivery_mode: if is_codex_app_server {
                "codex_app_server_turn"
            } else {
                "rust_boundary_codergen_turn"
            }
            .to_string(),
            reason: request.reason,
            message: if is_codex_app_server {
                "Intervention delivered to active Codex app-server turn."
            } else {
                "Intervention delivered to active Rust codergen session."
            }
            .to_string(),
            target_node_id: request.target_node_id,
        }
    }
}

fn rejected_intervention(
    request: &CodergenChildInterventionRequest,
    reason: impl Into<String>,
    delivery_mode: impl Into<String>,
    message: impl Into<String>,
) -> CodergenChildInterventionResult {
    CodergenChildInterventionResult {
        run_id: request.child_run_id.clone(),
        status: "rejected".to_string(),
        delivery_mode: delivery_mode.into(),
        reason: reason.into(),
        message: message.into(),
        target_node_id: request.target_node_id.clone(),
    }
}

fn intervention_steering_payload(
    request: &CodergenChildInterventionRequest,
    active: &ActiveCodergenSession,
) -> BTreeMap<String, Value> {
    let provider = request
        .provider
        .clone()
        .or_else(|| normalized_optional(Some(active.provider.as_str())));
    let model = request.model.clone().or_else(|| active.model.clone());
    let llm_profile = request
        .llm_profile
        .clone()
        .or_else(|| active.llm_profile.clone());
    let reasoning_effort = request
        .reasoning_effort
        .clone()
        .or_else(|| active.reasoning_effort.clone());
    BTreeMap::from([
        ("node_id".to_string(), json!(active.node_id.clone())),
        ("message".to_string(), json!(request.message.clone())),
        ("reason".to_string(), json!(request.reason.clone())),
        ("source".to_string(), json!(request.source.clone())),
        ("cycle".to_string(), json!(request.cycle)),
        (
            "child_run_id".to_string(),
            json!(request.child_run_id.clone()),
        ),
        (
            "parent_run_id".to_string(),
            json!(request.parent_run_id.clone()),
        ),
        (
            "parent_node_id".to_string(),
            json!(request.parent_node_id.clone()),
        ),
        (
            "root_run_id".to_string(),
            json!(request.root_run_id.clone()),
        ),
        (
            "target_node_id".to_string(),
            json!(request.target_node_id.clone()),
        ),
        ("provider".to_string(), json!(provider)),
        ("model".to_string(), json!(model)),
        ("llm_profile".to_string(), json!(llm_profile)),
        ("reasoning_effort".to_string(), json!(reasoning_effort)),
        (
            "project_path".to_string(),
            json!(active
                .project_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())),
        ),
        ("metadata".to_string(), json!(active.metadata.clone())),
    ])
}

fn default_manager_loop_source() -> String {
    "manager_loop".to_string()
}

fn normalized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CodergenBackendResponse {
    Text(String),
    Boolean(bool),
    Outcome(Outcome),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodergenBackendOutput {
    pub response: CodergenBackendResponse,
    #[serde(default)]
    pub events: Vec<CodergenEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl CodergenBackendOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            response: CodergenBackendResponse::Text(text.into()),
            events: Vec::new(),
            usage: None,
        }
    }

    pub fn boolean(value: bool) -> Self {
        Self {
            response: CodergenBackendResponse::Boolean(value),
            events: Vec::new(),
            usage: None,
        }
    }

    pub fn outcome(outcome: Outcome) -> Self {
        Self {
            response: CodergenBackendResponse::Outcome(outcome),
            events: Vec::new(),
            usage: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodergenEvent {
    pub event_type: String,
    #[serde(default)]
    pub payload: BTreeMap<String, Value>,
}

impl CodergenEvent {
    pub fn new(event_type: impl Into<String>, payload: BTreeMap<String, Value>) -> Self {
        Self {
            event_type: event_type.into(),
            payload,
        }
    }
}

pub trait CodergenBackend {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError>;
}

#[derive(Default)]
pub struct CodergenHandler {
    backend: Option<Box<dyn CodergenBackend>>,
}

impl CodergenHandler {
    pub fn simulation() -> Self {
        Self { backend: None }
    }

    pub fn with_backend(backend: impl CodergenBackend + 'static) -> Self {
        Self {
            backend: Some(Box::new(backend)),
        }
    }

    pub fn with_boxed_backend(backend: Box<dyn CodergenBackend>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    pub fn execute(
        &mut self,
        request: CodergenRequest,
    ) -> Result<CodergenExecution, CodergenError> {
        let stage_dir = ensure_stage_dir(request.logs_root.as_deref(), &request.node_id)?;
        let read_contract = resolve_context_read_contract(&request.node.attrs);
        if !read_contract.parse_error.is_empty() {
            let failure_reason = format!(
                "context.reads contract parse error: {}",
                read_contract.parse_error
            );
            let outcome = with_builtin_response_context(
                Outcome {
                    status: OutcomeStatus::Fail,
                    failure_reason: failure_reason.clone(),
                    retryable: Some(false),
                    failure_kind: Some(FailureKind::Contract),
                    ..Outcome::new(OutcomeStatus::Fail)
                },
                &request.node_id,
                &failure_reason,
            );
            write_stage_file(stage_dir.as_deref(), "response.md", &failure_reason)?;
            write_status_file(stage_dir.as_deref(), &outcome, 0, &[], None)?;
            return Ok(CodergenExecution::from_parts(
                outcome,
                String::new(),
                failure_reason,
                Vec::new(),
                0,
                Vec::new(),
                None,
            ));
        }

        let write_contract = resolve_context_write_contract(&request.node.attrs);
        let mut prompt = authored_prompt_for_node(&request.node);
        prompt = expand_goal(&prompt, &request.context, &request.graph);
        let response_contract = normalized_response_contract_name(&request.node.attrs);
        let contract_repair_attempts =
            contract_repair_attempts(&request.node.attrs, &response_contract);
        let runtime_mode = runtime_mode_for_node(&request.node.attrs);
        if response_contract == STATUS_ENVELOPE_RESPONSE_CONTRACT {
            prompt = format!(
                "{prompt}\n\n{}",
                build_status_envelope_prompt_appendix(Some(&write_contract))
            );
        }
        prompt = compose_prompt(&prompt, &request.context, &read_contract.declared_keys);
        write_stage_file(stage_dir.as_deref(), "prompt.md", &prompt)?;

        let resolution_inputs = resolution_inputs_for_request(&request);
        let mut metadata = request.metadata.clone();
        if codex_jsonrpc_trace_enabled() {
            if let Some(stage_dir) = stage_dir.as_ref() {
                metadata.insert(
                    CODEX_JSONRPC_TRACE_PATH_METADATA_KEY.to_string(),
                    json!(stage_dir
                        .join(CODEX_JSONRPC_TRACE_FILE_NAME)
                        .to_string_lossy()
                        .to_string()),
                );
            }
        }
        let backend_request = CodergenBackendRequest {
            node_id: request.node_id.clone(),
            prompt: prompt.clone(),
            context: request.context.clone(),
            response_contract: response_contract.clone(),
            contract_repair_attempts,
            timeout_seconds: timeout_seconds(&request.node.attrs),
            write_contract: write_contract.clone(),
            provider: resolve_effective_llm_provider(&resolution_inputs, &request.context),
            model: resolve_effective_llm_model(&resolution_inputs, &request.context),
            llm_profile: resolve_effective_llm_profile(&resolution_inputs, &request.context),
            reasoning_effort: resolve_effective_reasoning_effort(
                &resolution_inputs,
                &request.context,
            ),
            repair_attempt: None,
            runtime_mode,
            project_path: request.project_path.clone(),
            metadata,
        };

        let mut events = vec![event(
            "codergen_backend_request_started",
            [
                ("node_id", json!(request.node_id.clone())),
                ("response_contract", json!(response_contract.clone())),
                ("provider", json!(backend_request.provider.clone())),
                ("model", json!(backend_request.model.clone())),
                ("llm_profile", json!(backend_request.llm_profile.clone())),
                (
                    "reasoning_effort",
                    json!(backend_request.reasoning_effort.clone()),
                ),
                ("runtime_mode", json!(backend_request.runtime_mode.clone())),
            ],
        )];

        let (outcome, response_text, repair_attempts, violations, usage) = if self.backend.is_none()
        {
            let response_text = format!("[Simulated] Response for stage: {}", request.node_id);
            let outcome = with_builtin_response_context(
                Outcome {
                    status: OutcomeStatus::Success,
                    notes: format!("Stage completed: {}", request.node_id),
                    ..Outcome::new(OutcomeStatus::Success)
                },
                &request.node_id,
                &response_text,
            );
            (outcome, response_text, 0, Vec::new(), None)
        } else {
            self.run_backend_with_contract(
                backend_request,
                &write_contract,
                &mut events,
                &request.node_id,
            )?
        };

        write_stage_file(stage_dir.as_deref(), "response.md", &response_text)?;
        write_status_file(
            stage_dir.as_deref(),
            &outcome,
            repair_attempts,
            &violations,
            usage.as_ref(),
        )?;
        Ok(CodergenExecution::from_parts(
            outcome,
            prompt,
            response_text,
            events,
            repair_attempts,
            violations,
            usage,
        ))
    }

    fn run_backend_with_contract(
        &mut self,
        request: CodergenBackendRequest,
        write_contract: &ContextWriteContract,
        events: &mut Vec<CodergenEvent>,
        node_id: &str,
    ) -> Result<
        (
            Outcome,
            String,
            u32,
            Vec<StructuredContractViolation>,
            Option<Usage>,
        ),
        CodergenError,
    > {
        let mut current_request = request;
        let mut repair_attempts_used = 0;
        let mut violations = Vec::new();
        let mut usage = None;

        loop {
            let output = self
                .backend
                .as_mut()
                .expect("backend must exist")
                .run(current_request.clone())?;
            events.extend(output.events);
            if output.usage.is_some() {
                usage = output.usage;
            }
            match coerce_backend_response(
                output.response,
                &current_request.response_contract,
                write_contract,
                node_id,
            ) {
                CoercedBackendResponse::Accepted(outcome, response_text) => {
                    events.push(event(
                        "codergen_backend_response_accepted",
                        [
                            ("node_id", json!(node_id)),
                            ("repair_attempts", json!(repair_attempts_used)),
                        ],
                    ));
                    return Ok((
                        outcome,
                        response_text,
                        repair_attempts_used,
                        violations,
                        usage,
                    ));
                }
                CoercedBackendResponse::Violation(violation) => {
                    events.push(event(
                        "codergen_contract_violation",
                        [
                            ("node_id", json!(node_id)),
                            ("reason", json!(violation.reason.clone())),
                            ("repair_attempt", json!(repair_attempts_used)),
                        ],
                    ));
                    violations.push(violation.clone());
                    if repair_attempts_used >= current_request.contract_repair_attempts {
                        let outcome = with_builtin_response_context(
                            contract_failure_outcome(&violation),
                            node_id,
                            &violation.raw_text,
                        );
                        return Ok((
                            outcome,
                            violation.raw_text.clone(),
                            repair_attempts_used,
                            violations,
                            usage,
                        ));
                    }
                    repair_attempts_used += 1;
                    let repair_prompt = build_contract_repair_prompt(&violation);
                    events.push(event(
                        "codergen_contract_repair_attempt",
                        [
                            ("node_id", json!(node_id)),
                            ("attempt", json!(repair_attempts_used)),
                            (
                                "max_attempts",
                                json!(current_request.contract_repair_attempts),
                            ),
                        ],
                    ));
                    current_request.prompt = repair_prompt;
                    current_request.repair_attempt = Some(repair_attempts_used);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodergenExecution {
    pub outcome: Outcome,
    pub prompt: String,
    pub response_text: String,
    #[serde(default)]
    pub events: Vec<CodergenEvent>,
    #[serde(default)]
    pub repair_attempts: u32,
    #[serde(default)]
    pub contract_violations: Vec<StructuredContractViolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl CodergenExecution {
    fn from_parts(
        outcome: Outcome,
        prompt: String,
        response_text: String,
        events: Vec<CodergenEvent>,
        repair_attempts: u32,
        contract_violations: Vec<StructuredContractViolation>,
        usage: Option<Usage>,
    ) -> Self {
        Self {
            outcome,
            prompt,
            response_text,
            events,
            repair_attempts,
            contract_violations,
            usage,
        }
    }
}

enum CoercedBackendResponse {
    Accepted(Outcome, String),
    Violation(StructuredContractViolation),
}

fn coerce_backend_response(
    response: CodergenBackendResponse,
    response_contract: &str,
    write_contract: &ContextWriteContract,
    node_id: &str,
) -> CoercedBackendResponse {
    match response {
        CodergenBackendResponse::Outcome(outcome) => {
            let response_text = response_text_for_outcome(&outcome);
            if let Some(violation) = validate_write_contract_violation(
                &outcome,
                Some(write_contract),
                response_contract,
                &response_text,
            ) {
                return CoercedBackendResponse::Violation(violation);
            }
            CoercedBackendResponse::Accepted(
                with_builtin_response_context(outcome, node_id, &response_text),
                response_text,
            )
        }
        CodergenBackendResponse::Text(text) => {
            match coerce_structured_text_outcome(&text, response_contract) {
                StructuredTextOutcome::PlainText(result) => {
                    let response_text = result.raw_text;
                    CoercedBackendResponse::Accepted(
                        with_builtin_response_context(
                            Outcome {
                                status: OutcomeStatus::Success,
                                notes: response_text.clone(),
                                ..Outcome::new(OutcomeStatus::Success)
                            },
                            node_id,
                            &response_text,
                        ),
                        response_text,
                    )
                }
                StructuredTextOutcome::ModeledOutcome(result) => {
                    let response_text = result.outcome.raw_response_text.clone();
                    if let Some(violation) = validate_write_contract_violation(
                        &result.outcome,
                        Some(write_contract),
                        response_contract,
                        &response_text,
                    ) {
                        return CoercedBackendResponse::Violation(violation);
                    }
                    CoercedBackendResponse::Accepted(
                        with_builtin_response_context(result.outcome, node_id, &response_text),
                        response_text,
                    )
                }
                StructuredTextOutcome::ContractViolation(mut violation) => {
                    if violation.write_contract.is_none() {
                        violation.write_contract = Some(write_contract.clone());
                    }
                    CoercedBackendResponse::Violation(violation)
                }
                StructuredTextOutcome::InvalidOutcome(outcome) => {
                    let response_text = response_text_for_outcome(&outcome);
                    CoercedBackendResponse::Accepted(
                        with_builtin_response_context(outcome, node_id, &response_text),
                        response_text,
                    )
                }
            }
        }
        CodergenBackendResponse::Boolean(true) => {
            let response_text = "codergen backend success".to_string();
            CoercedBackendResponse::Accepted(
                with_builtin_response_context(
                    Outcome {
                        status: OutcomeStatus::Success,
                        notes: response_text.clone(),
                        ..Outcome::new(OutcomeStatus::Success)
                    },
                    node_id,
                    &response_text,
                ),
                response_text,
            )
        }
        CodergenBackendResponse::Boolean(false) => {
            let response_text = "codergen backend failure".to_string();
            CoercedBackendResponse::Accepted(
                with_builtin_response_context(
                    Outcome {
                        status: OutcomeStatus::Fail,
                        failure_reason: response_text.clone(),
                        ..Outcome::new(OutcomeStatus::Fail)
                    },
                    node_id,
                    &response_text,
                ),
                response_text,
            )
        }
    }
}

pub fn authored_prompt_for_node(node: &DotNode) -> String {
    let prompt = attr_string(&node.attrs, "prompt");
    if node_has_explicit_attr(node, "prompt") && !prompt.trim().is_empty() {
        return prompt.trim().to_string();
    }
    let label = attr_string(&node.attrs, "label");
    if node_has_explicit_attr(node, "label") && !label.trim().is_empty() {
        return label.trim().to_string();
    }
    String::new()
}

pub fn expand_goal(prompt: &str, context: &ContextMap, graph: &DotGraph) -> String {
    let goal = context
        .get("graph.goal")
        .map(context_value_to_text)
        .filter(|value| !value.is_empty())
        .or_else(|| attr_text(&graph.graph_attrs, "goal"))
        .unwrap_or_default();
    prompt.replace("$goal", &goal)
}

pub fn compose_prompt(prompt: &str, context: &ContextMap, declared_keys: &[String]) -> String {
    let mut sections = Vec::new();
    let carryover = context
        .get(RUNTIME_CONTEXT_CARRYOVER_KEY)
        .map(context_value_to_text)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !carryover.is_empty() {
        sections.push("Context carryover:".to_string());
        sections.push(carryover);
    }
    let declared_reads = render_declared_context_reads(context, declared_keys);
    if !declared_reads.is_empty() {
        sections.push("Declared context reads:".to_string());
        sections.push(declared_reads);
    }
    if sections.is_empty() {
        return prompt.to_string();
    }
    sections.push("Current stage task:".to_string());
    sections.push(prompt.to_string());
    sections.join("\n\n")
}

pub fn render_declared_context_reads(context: &ContextMap, declared_keys: &[String]) -> String {
    declared_keys
        .iter()
        .map(|key| {
            let value = context
                .get(key)
                .map(context_value_to_text)
                .unwrap_or_else(|| DECLARED_CONTEXT_MISSING_SENTINEL.to_string());
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolution_inputs_for_request(request: &CodergenRequest) -> LlmResolutionInputs {
    let reasoning_attr = request.node.attrs.get("reasoning_effort");
    LlmResolutionInputs {
        node_model: attr_text(&request.node.attrs, "llm_model"),
        node_provider: attr_text(&request.node.attrs, "llm_provider"),
        node_profile: attr_text(&request.node.attrs, "llm_profile"),
        node_reasoning_effort: attr_text(&request.node.attrs, "reasoning_effort"),
        node_reasoning_is_default_placeholder: reasoning_attr.is_some_and(|attr| attr.line == 0),
        fallback_model: request.fallback_model.clone(),
        fallback_provider: request.fallback_provider.clone(),
        fallback_profile: request.fallback_profile.clone(),
        fallback_reasoning_effort: request.fallback_reasoning_effort.clone(),
    }
}

fn normalized_response_contract_name(attrs: &BTreeMap<String, DotAttribute>) -> String {
    attr_text(attrs, "codergen.response_contract")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .replace('-', "_")
}

fn contract_repair_attempts(
    attrs: &BTreeMap<String, DotAttribute>,
    response_contract: &str,
) -> u32 {
    if response_contract.trim().is_empty() {
        return 0;
    }
    attr_text(attrs, "codergen.contract_repair_attempts")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .map(|value| value.max(0) as u32)
        .unwrap_or(DEFAULT_CONTRACT_REPAIR_ATTEMPTS)
}

fn runtime_mode_for_node(attrs: &BTreeMap<String, DotAttribute>) -> CodergenRuntimeMode {
    let mut runtime_mode = CodergenRuntimeMode {
        mode: attr_text(attrs, "codergen.runtime_mode")
            .or_else(|| attr_text(attrs, "codergen.execution_mode"))
            .map(|value| runtime_mode_kind(&value))
            .unwrap_or_default(),
        requires_tools: any_bool_attr(attrs, &["codergen.requires_tools", "codergen.tools"]),
        requires_steering: any_bool_attr(
            attrs,
            &["codergen.requires_steering", "codergen.steering"],
        ),
        requires_child_intervention: any_bool_attr(
            attrs,
            &[
                "codergen.requires_child_intervention",
                "codergen.child_intervention",
            ],
        ),
        requires_session_events: any_bool_attr(
            attrs,
            &[
                "codergen.requires_session_events",
                "codergen.session_events",
                "codergen.events",
            ],
        ),
    };
    if runtime_mode.requires_agent() {
        runtime_mode.mode = CodergenExecutionMode::Agent;
    }
    runtime_mode
}

fn runtime_mode_kind(value: &str) -> CodergenExecutionMode {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "agent" | "agent_required" | "session" | "session_backed" => CodergenExecutionMode::Agent,
        _ => CodergenExecutionMode::TextOnly,
    }
}

fn any_bool_attr(attrs: &BTreeMap<String, DotAttribute>, keys: &[&str]) -> bool {
    keys.iter().any(|key| attr_bool(attrs, key, false))
}

fn timeout_seconds(attrs: &BTreeMap<String, DotAttribute>) -> Option<f64> {
    let attr = attrs.get("timeout")?;
    match &attr.value {
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
        DotValue::String(value) => value.trim().parse().ok(),
        DotValue::Boolean(_) | DotValue::Null => None,
    }
}

fn with_builtin_response_context(
    mut outcome: Outcome,
    node_id: &str,
    response_text: &str,
) -> Outcome {
    let authored_updates = outcome.context_updates.clone();
    let mut merged = ContextMap::new();
    merged.insert("last_stage".to_string(), json!(node_id));
    merged.insert(
        "last_response".to_string(),
        json!(response_text.chars().take(200).collect::<String>()),
    );
    merged.extend(authored_updates);
    outcome.context_updates = merged;
    outcome
}

fn response_text_for_outcome(outcome: &Outcome) -> String {
    if !outcome.raw_response_text.is_empty() {
        return outcome.raw_response_text.clone();
    }
    if !outcome.notes.is_empty() {
        return outcome.notes.clone();
    }
    outcome.failure_reason.clone()
}

fn ensure_stage_dir(
    logs_root: Option<&Path>,
    node_id: &str,
) -> Result<Option<PathBuf>, CodergenError> {
    let Some(logs_root) = logs_root else {
        return Ok(None);
    };
    let stage_dir = logs_root.join(node_id);
    fs::create_dir_all(&stage_dir)
        .map_err(|source| CodergenError::Artifact(format!("create {stage_dir:?}: {source}")))?;
    Ok(Some(stage_dir))
}

fn write_stage_file(
    stage_dir: Option<&Path>,
    filename: &str,
    content: &str,
) -> Result<(), CodergenError> {
    let Some(stage_dir) = stage_dir else {
        return Ok(());
    };
    write_text_atomic(stage_dir.join(filename), format!("{content}\n"))
        .map_err(|source| CodergenError::Artifact(source.to_string()))
}

fn write_status_file(
    stage_dir: Option<&Path>,
    outcome: &Outcome,
    repair_attempts: u32,
    violations: &[StructuredContractViolation],
    usage: Option<&Usage>,
) -> Result<(), CodergenError> {
    let Some(stage_dir) = stage_dir else {
        return Ok(());
    };
    let mut payload = json!({
        "outcome": outcome.status.as_str(),
        "preferred_label": &outcome.preferred_label,
        "suggested_next_ids": &outcome.suggested_next_ids,
        "context_updates": &outcome.context_updates,
        "notes": &outcome.notes,
        "contract_repair_attempts": repair_attempts,
        "contract_violations": violations,
    });
    if let Some(failure_kind) = outcome.failure_kind {
        payload["failure_kind"] = json!(failure_kind.as_str());
    }
    if let Some(usage) = usage {
        payload["usage"] = json!(usage);
    }
    write_json_atomic(
        stage_dir.join("status.json"),
        &payload,
        JsonWriteOptions::default(),
    )
    .map_err(|source| CodergenError::Artifact(source.to_string()))
}

fn event<const N: usize>(event_type: &str, entries: [(&'static str, Value); N]) -> CodergenEvent {
    CodergenEvent {
        event_type: event_type.to_string(),
        payload: entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    }
}

fn context_value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| dot_value_text(&DotValue::Null))
        }
    }
}
