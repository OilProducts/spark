use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use unified_llm_adapter::Client;
use uuid::Uuid;

use crate::config::SessionConfig;
use crate::environment::{EnvironmentError, ExecutionEnvironment};
use crate::history::HistoryTurn;
use crate::profiles::ProviderProfile;
use crate::session::{LlmClientHandle, Session, SessionAbortHandle, SessionState};
use crate::tools::ToolExecutionOutput;

const SUBAGENT_TASK_METADATA_KEY: &str = "task";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Closed,
}

impl Default for SubAgentStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub handle_id: Uuid,
    pub status: SubAgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub turns_used: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl SubAgentResult {
    pub fn new(handle_id: Uuid, status: SubAgentStatus) -> Self {
        Self {
            handle_id,
            status,
            session_id: None,
            output: None,
            success: status == SubAgentStatus::Completed,
            turns_used: 0,
            response_id: None,
            summary: None,
            error: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentHandle {
    pub id: Uuid,
    #[serde(default)]
    pub status: SubAgentStatus,
    #[serde(default, skip)]
    pub session: Option<Box<Session>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<ProviderProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SubAgentResult>,
}

impl PartialEq for SubAgentHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.status == other.status
            && self.session_id == other.session_id
            && self.provider_profile == other.provider_profile
            && self.working_directory == other.working_directory
            && self.metadata == other.metadata
            && self.result == other.result
    }
}

impl SubAgentHandle {
    pub fn new(session: Session) -> Self {
        let id = session.id;
        let provider_profile = session.provider_profile.clone();
        let working_directory = PathBuf::from(session.execution_environment.working_directory());
        Self {
            id,
            status: SubAgentStatus::Pending,
            session: Some(Box::new(session)),
            session_id: Some(id),
            provider_profile: Some(provider_profile),
            working_directory: Some(working_directory),
            metadata: BTreeMap::new(),
            result: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct SubAgentWorker {
    state: Arc<(Mutex<SubAgentWorkerState>, Condvar)>,
    join_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    abort_handle: SessionAbortHandle,
}

#[derive(Debug, Clone)]
struct SubAgentWorkerState {
    status: SubAgentStatus,
    result: Option<SubAgentResult>,
    session: Option<Session>,
    queued_inputs: VecDeque<String>,
    close_requested: bool,
    wait_requested: bool,
}

impl fmt::Debug for SubAgentWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.0.lock().expect("subagent worker state");
        formatter
            .debug_struct("SubAgentWorker")
            .field("status", &state.status)
            .field("has_result", &state.result.is_some())
            .field("queued_inputs", &state.queued_inputs.len())
            .field("close_requested", &state.close_requested)
            .field("wait_requested", &state.wait_requested)
            .finish()
    }
}

impl PartialEq for SubAgentWorker {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl SubAgentWorker {
    fn start(
        mut session: Session,
        client: Client,
        task: String,
        handle_id: Uuid,
        session_id: Option<Uuid>,
        metadata: BTreeMap<String, Value>,
    ) -> Result<Self, std::io::Error> {
        let abort_handle = session.abort_handle();
        let state = Arc::new((
            Mutex::new(SubAgentWorkerState {
                status: SubAgentStatus::Running,
                result: None,
                session: Some(session.clone()),
                queued_inputs: VecDeque::new(),
                close_requested: false,
                wait_requested: false,
            }),
            Condvar::new(),
        ));
        let worker_state = state.clone();
        let join_handle = thread::Builder::new()
            .name(format!("spark-subagent-{handle_id}"))
            .spawn(move || {
                run_subagent_worker(
                    &mut session,
                    client,
                    task,
                    handle_id,
                    session_id,
                    metadata,
                    worker_state,
                );
            })?;

        Ok(Self {
            state,
            join_handle: Arc::new(Mutex::new(Some(join_handle))),
            abort_handle,
        })
    }

    fn queue_input(&self, message: String) -> Result<(), SubAgentStatus> {
        let (state, condvar) = &*self.state;
        let mut state = state.lock().expect("subagent worker state");
        if state.result.is_some() || state.status != SubAgentStatus::Running {
            return Err(state.status);
        }
        if state.close_requested {
            return Err(SubAgentStatus::Closed);
        }
        state.queued_inputs.push_back(message);
        condvar.notify_all();
        Ok(())
    }

    fn wait(&self) {
        {
            let (state, condvar) = &*self.state;
            let mut state = state.lock().expect("subagent worker state");
            state.wait_requested = true;
            condvar.notify_all();
        }
        let join_handle = self
            .join_handle
            .lock()
            .expect("subagent join handle")
            .take();
        if let Some(join_handle) = join_handle {
            let _ = join_handle.join();
        }
    }

    fn close(&self, handle_id: Uuid, session_id: Option<Uuid>, metadata: &BTreeMap<String, Value>) {
        let (state, condvar) = &*self.state;
        {
            let mut state = state.lock().expect("subagent worker state");
            state.close_requested = true;
            if state.result.is_none() {
                state.status = SubAgentStatus::Closed;
                state.wait_requested = true;
                state.result = Some(subagent_result_from_parts(
                    handle_id,
                    session_id,
                    SubAgentStatus::Closed,
                    None,
                    metadata,
                    state.session.as_ref(),
                ));
            }
            condvar.notify_all();
        }
        self.abort_handle.abort_with_reason("child agent closed");
    }

    fn snapshot(&self) -> SubAgentWorkerState {
        self.state.0.lock().expect("subagent worker state").clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChildSessionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl ChildSessionOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_working_dir(mut self, working_dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }

    pub fn with_metadata(mut self, metadata: BTreeMap<String, Value>) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Clone)]
pub struct SubAgentToolRuntime {
    state: Arc<Mutex<SubAgentToolRuntimeState>>,
}

struct SubAgentToolRuntimeState {
    parent_provider_profile: ProviderProfile,
    parent_environment: ExecutionEnvironment,
    parent_config: SessionConfig,
    parent_client_handle: LlmClientHandle,
    client: Client,
    active_subagents: BTreeMap<String, SubAgentHandle>,
    active_subagent_workers: BTreeMap<String, SubAgentWorker>,
}

impl SubAgentToolRuntime {
    pub(crate) fn from_session(parent_session: &mut Session, client: Client) -> Self {
        Self {
            state: Arc::new(Mutex::new(SubAgentToolRuntimeState {
                parent_provider_profile: parent_session.provider_profile.clone(),
                parent_environment: parent_session.execution_environment.clone(),
                parent_config: parent_session.config.clone(),
                parent_client_handle: parent_session.llm_client.clone(),
                client,
                active_subagents: std::mem::take(&mut parent_session.active_subagents),
                active_subagent_workers: std::mem::take(
                    &mut parent_session.active_subagent_workers,
                ),
            })),
        }
    }

    pub(crate) fn restore_into(&self, parent_session: &mut Session) {
        let state = self.state.lock().expect("subagent runtime state");
        parent_session.active_subagents = state.active_subagents.clone();
        parent_session.active_subagent_workers = state.active_subagent_workers.clone();
    }

    pub fn active_subagents(&self) -> BTreeMap<String, SubAgentHandle> {
        self.state
            .lock()
            .expect("subagent runtime state")
            .active_subagents
            .clone()
    }

    pub fn spawn_agent(&self, arguments: &Value) -> ToolExecutionOutput {
        let Some(task) = required_string_argument(arguments, "task") else {
            return tool_error("Missing required argument: task");
        };
        let working_dir = match optional_path_argument(arguments, "working_dir") {
            Ok(value) => value,
            Err(message) => return tool_error(message),
        };
        let model = match optional_string_argument(arguments, "model") {
            Ok(value) => value,
            Err(message) => return tool_error(message),
        };
        let max_turns = match optional_u32_argument(arguments, "max_turns") {
            Ok(value) => value,
            Err(message) => return tool_error(message),
        };

        let mut state = self.state.lock().expect("subagent runtime state");
        if let Some(model) = model.as_deref() {
            if !state
                .parent_provider_profile
                .allows_subagent_model_override(model)
            {
                return tool_error(model_override_error(&state.parent_provider_profile, model));
            }
        }

        let metadata =
            BTreeMap::from([(SUBAGENT_TASK_METADATA_KEY.to_string(), json!(task.clone()))]);
        let options = ChildSessionOptions {
            working_dir,
            model,
            max_turns,
            metadata,
        };
        let mut handle = match build_child_handle_from_parts(
            &state.parent_provider_profile,
            &state.parent_environment,
            &state.parent_config,
            &state.parent_client_handle,
            options,
        ) {
            Ok(handle) => handle,
            Err(error) => return tool_error(format!("Failed to spawn child agent: {error}")),
        };
        let worker_session = match handle.session.as_deref().cloned() {
            Some(session) => session,
            None => return tool_error("Failed to spawn child agent: child session is unavailable"),
        };
        let worker = match SubAgentWorker::start(
            worker_session,
            state.client.clone(),
            task,
            handle.id,
            handle.session_id,
            handle.metadata.clone(),
        ) {
            Ok(worker) => worker,
            Err(error) => return tool_error(format!("Failed to spawn child agent: {error}")),
        };
        handle.status = SubAgentStatus::Running;
        let payload = subagent_handle_payload(&handle);
        let agent_id = handle.id.to_string();
        state
            .active_subagent_workers
            .insert(agent_id.clone(), worker);
        state.active_subagents.insert(agent_id, handle);
        ToolExecutionOutput::success(payload)
    }

    pub fn send_input(&self, arguments: &Value) -> ToolExecutionOutput {
        let Some(agent_id) = required_string_argument(arguments, "agent_id") else {
            return tool_error("Missing required argument: agent_id");
        };
        let Some(message) = required_string_argument(arguments, "message") else {
            return tool_error("Missing required argument: message");
        };

        let mut state = self.state.lock().expect("subagent runtime state");
        let worker = state.active_subagent_workers.get(&agent_id).cloned();
        let Some(handle) = state.active_subagents.get_mut(&agent_id) else {
            return tool_error(format!("Unknown child agent: {agent_id}"));
        };
        sync_worker_state(handle, worker.as_ref());
        if let Some(result) = handle.result.as_ref() {
            return tool_error(format!(
                "Child agent is {}: {agent_id}",
                subagent_status_value(result.status)
            ));
        }
        if handle.status != SubAgentStatus::Running {
            return tool_error(format!(
                "Child agent is {}: {agent_id}",
                subagent_status_value(handle.status)
            ));
        }
        let Some(worker) = worker.as_ref() else {
            match handle.session.as_deref_mut() {
                Some(child_session) => child_session.queue_follow_up(message),
                None => {
                    handle.status = SubAgentStatus::Failed;
                    handle.result = Some(subagent_result(
                        handle,
                        SubAgentStatus::Failed,
                        Some("child session is unavailable".to_string()),
                    ));
                    return tool_error(format!("Child agent is failed: {agent_id}"));
                }
            }
            return ToolExecutionOutput::success(subagent_handle_payload(handle));
        };
        if let Err(status) = worker.queue_input(message) {
            sync_worker_state(handle, Some(worker));
            return tool_error(format!(
                "Child agent is {}: {agent_id}",
                subagent_status_value(status)
            ));
        }
        ToolExecutionOutput::success(subagent_handle_payload(handle))
    }

    pub fn wait(&self, arguments: &Value) -> ToolExecutionOutput {
        let Some(agent_id) = required_string_argument(arguments, "agent_id") else {
            return tool_error("Missing required argument: agent_id");
        };

        let mut state = self.state.lock().expect("subagent runtime state");
        let client = state.client.clone();
        let worker = state.active_subagent_workers.get(&agent_id).cloned();
        let Some(handle) = state.active_subagents.get_mut(&agent_id) else {
            return tool_error(format!("Unknown child agent: {agent_id}"));
        };
        sync_worker_state(handle, worker.as_ref());
        if let Some(result) = handle.result.clone() {
            return wait_result_output(&result);
        }
        if let Some(worker) = worker {
            worker.wait();
            sync_worker_state(handle, Some(&worker));
            let result = handle
                .result
                .clone()
                .unwrap_or_else(|| subagent_result(handle, handle.status, None));
            handle.result = Some(result.clone());
            return wait_result_output(&result);
        }
        if matches!(
            handle.status,
            SubAgentStatus::Completed | SubAgentStatus::Failed | SubAgentStatus::Closed
        ) {
            let result = subagent_result(handle, handle.status, None);
            handle.result = Some(result.clone());
            return wait_result_output(&result);
        }

        let task = handle
            .metadata
            .get(SUBAGENT_TASK_METADATA_KEY)
            .and_then(Value::as_str)
            .map(str::to_string);
        let error = match (task, handle.session.as_deref_mut()) {
            (Some(task), Some(child_session)) => {
                handle.status = SubAgentStatus::Running;
                child_session
                    .process_input(&client, task)
                    .err()
                    .map(|error| error.message)
            }
            (None, _) => Some("child task is unavailable".to_string()),
            (_, None) => Some("child session is unavailable".to_string()),
        };
        let status = if error.is_some() {
            SubAgentStatus::Failed
        } else {
            SubAgentStatus::Completed
        };
        handle.status = status;
        let result = subagent_result(handle, status, error);
        handle.result = Some(result.clone());
        wait_result_output(&result)
    }

    pub fn close_agent(&self, arguments: &Value) -> ToolExecutionOutput {
        let Some(agent_id) = required_string_argument(arguments, "agent_id") else {
            return tool_error("Missing required argument: agent_id");
        };

        let mut state = self.state.lock().expect("subagent runtime state");
        let worker = state.active_subagent_workers.get(&agent_id).cloned();
        let Some(handle) = state.active_subagents.get_mut(&agent_id) else {
            return tool_error(format!("Unknown child agent: {agent_id}"));
        };
        sync_worker_state(handle, worker.as_ref());
        let result = if let Some(result) = handle.result.clone() {
            result
        } else if let Some(worker) = worker.as_ref() {
            worker.close(handle.id, handle.session_id, &handle.metadata);
            sync_worker_state(handle, Some(worker));
            handle
                .result
                .clone()
                .unwrap_or_else(|| subagent_result(handle, SubAgentStatus::Closed, None))
        } else {
            if let Some(child_session) = handle.session.as_deref_mut() {
                child_session.close();
            }
            handle.status = SubAgentStatus::Closed;
            subagent_result(handle, SubAgentStatus::Closed, None)
        };
        handle.status = result.status;
        handle.result = Some(result.clone());
        ToolExecutionOutput::success(subagent_result_payload(&result))
    }
}

impl fmt::Debug for SubAgentToolRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubAgentToolRuntime")
            .field("active_subagents", &self.active_subagents().len())
            .finish()
    }
}

impl PartialEq for SubAgentToolRuntime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("max_subagent_depth exceeded")]
pub struct SubAgentLimitError {
    pub max_subagent_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("working_dir must remain within the parent environment: {working_dir}")]
pub struct SubAgentWorkingDirectoryError {
    pub working_dir: PathBuf,
    pub parent_working_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown child agent: {agent_id}")]
pub struct SubAgentLookupError {
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("child agent {agent_id} is {status:?}")]
pub struct SubAgentStateError {
    pub agent_id: String,
    pub status: SubAgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubAgentError {
    #[error(transparent)]
    Limit(#[from] SubAgentLimitError),
    #[error(transparent)]
    WorkingDirectory(#[from] SubAgentWorkingDirectoryError),
    #[error(transparent)]
    Lookup(#[from] SubAgentLookupError),
    #[error(transparent)]
    State(#[from] SubAgentStateError),
}

impl SubAgentError {
    pub fn recoverable(&self) -> bool {
        true
    }
}

pub type SubAgentRuntimeResult<T> = Result<T, SubAgentError>;

pub fn is_subagent_tool_name(name: &str) -> bool {
    matches!(name, "spawn_agent" | "send_input" | "wait" | "close_agent")
}

pub fn create_child_session(
    parent_session: &mut Session,
    options: ChildSessionOptions,
) -> SubAgentRuntimeResult<&SubAgentHandle> {
    parent_session.create_child_session(options)
}

pub(crate) fn close_active_subagents(
    active_subagents: &mut BTreeMap<String, SubAgentHandle>,
    active_subagent_workers: &mut BTreeMap<String, SubAgentWorker>,
) {
    for (agent_id, handle) in active_subagents.iter_mut() {
        let worker = active_subagent_workers.get(agent_id);
        close_subagent_handle(handle, worker);
    }
    active_subagents.clear();
    active_subagent_workers.clear();
}

impl Session {
    pub fn create_child_session(
        &mut self,
        options: ChildSessionOptions,
    ) -> SubAgentRuntimeResult<&SubAgentHandle> {
        let handle = build_child_handle(self, options)?;
        let id = handle.id.to_string();
        self.active_subagents.insert(id.clone(), handle);
        Ok(self
            .active_subagents
            .get(&id)
            .expect("subagent handle was just inserted"))
    }

    pub fn active_subagent(&self, agent_id: impl AsRef<str>) -> Option<&SubAgentHandle> {
        self.active_subagents.get(agent_id.as_ref())
    }

    pub fn active_subagent_mut(
        &mut self,
        agent_id: impl AsRef<str>,
    ) -> Option<&mut SubAgentHandle> {
        self.active_subagents.get_mut(agent_id.as_ref())
    }
}

fn build_child_handle(
    parent_session: &Session,
    options: ChildSessionOptions,
) -> SubAgentRuntimeResult<SubAgentHandle> {
    build_child_handle_from_parts(
        &parent_session.provider_profile,
        &parent_session.execution_environment,
        &parent_session.config,
        &parent_session.llm_client,
        options,
    )
}

fn build_child_handle_from_parts(
    parent_profile: &ProviderProfile,
    parent_environment: &ExecutionEnvironment,
    parent_config: &SessionConfig,
    parent_client_handle: &LlmClientHandle,
    options: ChildSessionOptions,
) -> SubAgentRuntimeResult<SubAgentHandle> {
    if parent_config.max_subagent_depth == 0 {
        return Err(SubAgentLimitError {
            max_subagent_depth: parent_config.max_subagent_depth,
        }
        .into());
    }

    let child_environment = child_environment(parent_environment, options.working_dir.as_ref())?;
    let mut child_profile = parent_profile.clone();
    if let Some(model) = options.model.as_deref() {
        child_profile.model = model.to_string();
    }
    let child_config = child_config(parent_config, options.max_turns);
    let mut child_session = Session::new(child_profile, child_environment, child_config);
    child_session.llm_client = parent_client_handle.clone();

    let mut handle = SubAgentHandle::new(child_session);
    handle.metadata = options.metadata;
    Ok(handle)
}

fn run_subagent_worker(
    session: &mut Session,
    client: Client,
    task: String,
    handle_id: Uuid,
    session_id: Option<Uuid>,
    metadata: BTreeMap<String, Value>,
    worker_state: Arc<(Mutex<SubAgentWorkerState>, Condvar)>,
) {
    let mut current_input = Some(task);
    let mut final_status = SubAgentStatus::Completed;
    let mut final_error = None;

    'worker: while let Some(input) = current_input.take() {
        if worker_close_requested(&worker_state) {
            final_status = SubAgentStatus::Closed;
            break;
        }

        match session.process_input(&client, input) {
            Ok(()) => {
                sync_worker_session(&worker_state, session);
            }
            Err(error) => {
                final_status = SubAgentStatus::Failed;
                final_error = Some(error.message);
                break;
            }
        }

        let (state, condvar) = &*worker_state;
        let mut state = state.lock().expect("subagent worker state");
        state.session = Some(session.clone());
        loop {
            if state.close_requested || session.state == SessionState::Closed {
                final_status = SubAgentStatus::Closed;
                break 'worker;
            }
            if let Some(next_input) = state.queued_inputs.pop_front() {
                state.status = SubAgentStatus::Running;
                current_input = Some(next_input);
                break;
            }
            if state.wait_requested {
                final_status = SubAgentStatus::Completed;
                break 'worker;
            }
            state.status = SubAgentStatus::Running;
            state = condvar.wait(state).expect("subagent worker state");
        }
    }

    let result = subagent_result_from_parts(
        handle_id,
        session_id,
        final_status,
        final_error,
        &metadata,
        Some(session),
    );
    let (state, condvar) = &*worker_state;
    let mut state = state.lock().expect("subagent worker state");
    state.session = Some(session.clone());
    if state.close_requested {
        state.status = SubAgentStatus::Closed;
        if state.result.is_none() {
            state.result = Some(subagent_result_from_parts(
                handle_id,
                session_id,
                SubAgentStatus::Closed,
                None,
                &metadata,
                Some(session),
            ));
        }
    } else {
        state.status = result.status;
        state.result = Some(result);
    }
    condvar.notify_all();
}

fn worker_close_requested(worker_state: &Arc<(Mutex<SubAgentWorkerState>, Condvar)>) -> bool {
    worker_state
        .0
        .lock()
        .expect("subagent worker state")
        .close_requested
}

fn sync_worker_session(
    worker_state: &Arc<(Mutex<SubAgentWorkerState>, Condvar)>,
    session: &Session,
) {
    worker_state
        .0
        .lock()
        .expect("subagent worker state")
        .session = Some(session.clone());
}

fn sync_worker_state(handle: &mut SubAgentHandle, worker: Option<&SubAgentWorker>) {
    let Some(worker) = worker else {
        return;
    };
    let snapshot = worker.snapshot();
    handle.status = snapshot.status;
    if let Some(session) = snapshot.session {
        handle.session = Some(Box::new(session));
    }
    if let Some(result) = snapshot.result {
        handle.status = result.status;
        handle.result = Some(result);
    }
}

fn close_subagent_handle(handle: &mut SubAgentHandle, worker: Option<&SubAgentWorker>) {
    sync_worker_state(handle, worker);
    if handle.result.is_some() {
        return;
    }
    if let Some(worker) = worker {
        worker.close(handle.id, handle.session_id, &handle.metadata);
        sync_worker_state(handle, Some(worker));
        return;
    }
    if let Some(child_session) = handle.session.as_deref_mut() {
        child_session.close();
    }
    handle.status = SubAgentStatus::Closed;
    handle.result = Some(subagent_result(handle, SubAgentStatus::Closed, None));
}

fn child_environment(
    parent_environment: &ExecutionEnvironment,
    working_dir: Option<&PathBuf>,
) -> SubAgentRuntimeResult<ExecutionEnvironment> {
    let Some(working_dir) = working_dir else {
        return Ok(parent_environment.clone());
    };
    parent_environment
        .scoped_child(working_dir)
        .map_err(|error| match error {
            EnvironmentError::PermissionDenied(_) => SubAgentWorkingDirectoryError {
                working_dir: working_dir.clone(),
                parent_working_dir: PathBuf::from(parent_environment.working_directory()),
            }
            .into(),
            EnvironmentError::InvalidInput(_) => SubAgentWorkingDirectoryError {
                working_dir: working_dir.clone(),
                parent_working_dir: PathBuf::from(parent_environment.working_directory()),
            }
            .into(),
            _ => SubAgentWorkingDirectoryError {
                working_dir: working_dir.clone(),
                parent_working_dir: PathBuf::from(parent_environment.working_directory()),
            }
            .into(),
        })
}

fn child_config(parent_config: &SessionConfig, max_turns: Option<u32>) -> SessionConfig {
    let mut child_config = parent_config.clone();
    child_config.max_turns = max_turns.unwrap_or(0);
    child_config.max_subagent_depth = parent_config.max_subagent_depth.saturating_sub(1);
    child_config
}

fn required_string_argument(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .as_object()
        .and_then(|object| object.get(name))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_string_argument(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    let Some(value) = arguments.as_object().and_then(|object| object.get(name)) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(format!("{name} must be a string"));
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    Ok(Some(value.to_string()))
}

fn optional_path_argument(arguments: &Value, name: &str) -> Result<Option<PathBuf>, String> {
    optional_string_argument(arguments, name).map(|value| value.map(PathBuf::from))
}

fn optional_u32_argument(arguments: &Value, name: &str) -> Result<Option<u32>, String> {
    let Some(value) = arguments.as_object().and_then(|object| object.get(name)) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64().and_then(|value| u32::try_from(value).ok()) else {
        return Err(format!(
            "{name} must be a non-negative integer no larger than u32::MAX"
        ));
    };
    Ok(Some(value))
}

fn model_override_error(parent_profile: &ProviderProfile, requested_model: &str) -> String {
    let provider = parent_profile
        .request_provider_id()
        .unwrap_or_else(|| "<unspecified>".to_string());
    let parent_model = if parent_profile.model.trim().is_empty() {
        "<unspecified>"
    } else {
        parent_profile.model.as_str()
    };
    let allowed_models = parent_profile.allowed_subagent_model_overrides();
    format!(
        "model override is not allowed for spawn_agent: requested_model={requested_model:?}, parent_model={parent_model:?}, provider={provider:?}, allowed_models={allowed_models:?}. Retry without the model argument or choose an allowed model."
    )
}

fn tool_error(message: impl Into<String>) -> ToolExecutionOutput {
    ToolExecutionOutput::error(Value::String(message.into()))
}

fn wait_result_output(result: &SubAgentResult) -> ToolExecutionOutput {
    let payload = subagent_result_payload(result);
    if result.status == SubAgentStatus::Failed || result.error.is_some() {
        ToolExecutionOutput::error(payload)
    } else {
        ToolExecutionOutput::success(payload)
    }
}

fn subagent_handle_payload(handle: &SubAgentHandle) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("agent_id".to_string(), Value::String(handle.id.to_string())),
        (
            "status".to_string(),
            Value::String(subagent_status_value(handle.status).to_string()),
        ),
    ]);
    if let Some(session_id) = handle.session_id {
        payload.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );
    }
    if let Some(working_directory) = handle.working_directory.as_ref() {
        payload.insert(
            "working_dir".to_string(),
            Value::String(working_directory.to_string_lossy().to_string()),
        );
    }
    if let Some(provider_profile) = handle.provider_profile.as_ref() {
        payload.insert(
            "model".to_string(),
            Value::String(provider_profile.model.clone()),
        );
    }
    Value::Object(payload)
}

fn subagent_result_payload(result: &SubAgentResult) -> Value {
    json!({
        "agent_id": result.handle_id.to_string(),
        "session_id": result.session_id.map(|session_id| session_id.to_string()),
        "status": subagent_status_value(result.status),
        "success": result.success,
        "output": result.output,
        "turns_used": result.turns_used,
        "response_id": result.response_id,
        "summary": result.summary,
        "error": result.error,
        "metadata": result.metadata,
    })
}

fn subagent_result(
    handle: &SubAgentHandle,
    status: SubAgentStatus,
    error: Option<String>,
) -> SubAgentResult {
    subagent_result_from_parts(
        handle.id,
        handle.session_id,
        status,
        error,
        &handle.metadata,
        handle.session.as_deref(),
    )
}

fn subagent_result_from_parts(
    handle_id: Uuid,
    session_id: Option<Uuid>,
    status: SubAgentStatus,
    error: Option<String>,
    metadata: &BTreeMap<String, Value>,
    session: Option<&Session>,
) -> SubAgentResult {
    let mut result = SubAgentResult::new(handle_id, status);
    result.session_id = session_id;
    result.error = error;
    result.success = status == SubAgentStatus::Completed && result.error.is_none();
    result.metadata = metadata.clone();
    if let Some(session) = session {
        result.turns_used = session.history.len();
        if let Some((output, response_id)) = latest_child_response(session) {
            result.output = Some(output.clone());
            result.summary = Some(output);
            result.response_id = response_id;
        }
    }
    result
}

fn latest_child_response(session: &Session) -> Option<(String, Option<String>)> {
    session.history.iter().rev().find_map(|turn| match turn {
        HistoryTurn::Assistant(assistant) => {
            let output = assistant.text();
            (!output.is_empty()).then(|| (output, assistant.response_id.clone()))
        }
        _ => None,
    })
}

fn subagent_status_value(status: SubAgentStatus) -> &'static str {
    match status {
        SubAgentStatus::Pending => "pending",
        SubAgentStatus::Running => "running",
        SubAgentStatus::Completed => "completed",
        SubAgentStatus::Failed => "failed",
        SubAgentStatus::Closed => "closed",
    }
}
