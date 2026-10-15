//! Per-turn stdio control state. No process or pending question survives a turn.
use super::*;
use std::collections::BTreeSet;
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::agent::AgentRequestUserInputAnswerRequest;

const NO_QUESTION_HANDLER: &str =
    "No one can answer questions during this turn. Proceed on your best judgment and state the assumptions you made.";
const PLAN_SUBMITTED: &str =
    "Your plan was submitted to the user for review. Stop here; do not implement it.";
const PLAN_MISSING: &str = "ExitPlanMode needs your plan in `plan`.";
const CONTEXT_USAGE_TIMEOUT: Duration = Duration::from_secs(5);

type Key = (String, String);
type Registry = Mutex<BTreeMap<Key, Weak<Mutex<Control>>>>;
fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// Who answers the turn's `AskUserQuestion` calls.
#[derive(Clone)]
pub(super) enum Questions {
    Deny,
    /// The chat request-user-input route.
    Chat,
    /// The flow's clarification handler, the path Codex nodes use.
    Flow(crate::codergen::ClarificationHandler),
}

pub(super) struct Control {
    pub child: Child,
    stdin: Option<Sender<String>>,
    questions: Questions,
    /// A plan-mode turn: `ExitPlanMode` submits the turn's plan.
    plan_mode: bool,
    /// Flow clarifications awaiting answers, by request id.
    clarifications: Vec<(String, crate::codergen::ClarificationPoll)>,
    enabled: bool,
    initialized: bool,
    pending: BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    interrupt: Option<String>,
    interrupted: bool,
    /// Stop was requested; the next result ends the turn.
    pub stopping: bool,
    /// Live non-ambient background tasks, from the last `background_tasks_changed`.
    tasks: usize,
    /// Messages sent after the prompt whose uuid no result has confirmed
    /// yet, by uuid, with their caller's message id.
    pub undelivered: BTreeMap<String, String>,
    /// Messages written mid-turn, as `(message_id, message)`, not yet
    /// reported. Recorded with the write, so the turn reports each before
    /// any output read after it.
    pub sent: Vec<(String, String)>,
    /// A result left only background tasks running and no main model turn
    /// has started since.
    idle: bool,
    /// The `get_context_usage` request sent after the final result, with
    /// when it gives up; the turn ends at its answer.
    context_request: Option<(String, std::time::Instant)>,
    /// `{used_tokens, window_tokens}` from the answered request.
    pub context: Option<Value>,
    pub ended: bool,
    pub logs: Vec<AgentRawLogLine>,
}

impl Control {
    pub fn register(
        child: Child,
        stdin: ChildStdin,
        request: &AgentTurnRequest,
        questions: Questions,
    ) -> Arc<Mutex<Self>> {
        let control = Arc::new(Mutex::new(Self {
            child,
            stdin: Some(crate::agent::stdin_writer(stdin)),
            questions,
            plan_mode: plan_mode(request),
            clarifications: Vec::new(),
            enabled: false,
            initialized: false,
            pending: BTreeMap::new(),
            seen: BTreeSet::new(),
            interrupt: None,
            interrupted: false,
            stopping: false,
            tasks: 0,
            undelivered: BTreeMap::new(),
            sent: Vec::new(),
            idle: false,
            context_request: None,
            context: None,
            ended: false,
            logs: Vec::new(),
        }));
        let mut entries = registry().lock().unwrap();
        entries.retain(|_, entry| entry.strong_count() > 0);
        entries.insert(
            (
                request.project_path.clone(),
                request.conversation_id.clone(),
            ),
            Arc::downgrade(&control),
        );
        control
    }

    pub fn interrupted_result(&self, message: &Value) -> bool {
        self.interrupted
            && message["type"] == "result"
            && matches!(
                message["terminal_reason"].as_str(),
                Some("aborted_streaming" | "aborted_tools")
            )
    }

    /// The final result has arrived; the turn ends once the context usage
    /// request is answered or times out.
    pub fn finished(&self) -> bool {
        self.ended || self.context_request.is_some()
    }

    /// Ends a turn still waiting on its context usage once it is overdue or
    /// the process has exited. Silent: the result's context window stands.
    pub fn expire_context_request(&mut self, exited: bool) {
        if self
            .context_request
            .as_ref()
            .is_some_and(|(_, deadline)| exited || std::time::Instant::now() >= *deadline)
        {
            self.context_request = None;
            self.end();
        }
    }

    /// Asks the still-open session how full its context is. No more
    /// messages or interrupts go in meanwhile. False if it cannot be asked.
    fn request_context_usage(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.enabled = false;
        let id = Uuid::new_v4().to_string();
        if !self.send(json!({"type": "control_request", "request_id": id, "request": {"subtype": "get_context_usage"}})) {
            return false;
        }
        self.context_request = Some((id, std::time::Instant::now() + CONTEXT_USAGE_TIMEOUT));
        true
    }

    /// Ends the turn: no more input, answers or interrupts reach the process.
    pub fn end(&mut self) {
        self.ended = true;
        self.one_shot();
    }

    /// Closes stdin so the CLI ends at its result, as it does without control.
    /// Pending questions are denied first: the CLI waits on every request.
    pub fn one_shot(&mut self) {
        self.enabled = false;
        self.clarifications.clear();
        for id in std::mem::take(&mut self.pending).into_keys() {
            self.deny(
                &id,
                "No one can answer this question now; proceed with your best judgment.",
            );
        }
        self.stdin.take();
    }

    pub fn log(&mut self, reason: &str) {
        tracing::warn!(reason, "Claude Code control traffic");
        self.logs.push(AgentRawLogLine {
            direction: "control".into(),
            line: format!("Claude Code control: {reason}"),
        });
    }

    fn send(&mut self, message: Value) -> bool {
        let line = message.to_string();
        let sent = self
            .stdin
            .as_ref()
            .is_some_and(|stdin| stdin.send(line.clone()).is_ok());
        if !sent {
            self.log("stdin write failed");
            self.one_shot();
            return false;
        }
        self.logs.push(AgentRawLogLine {
            direction: "stdin".into(),
            line,
        });
        true
    }

    /// Writes a user message stamped with a uuid. A message after the
    /// prompt, named by `message_id`, holds the turn open until a result
    /// lists it as delivered.
    pub fn send_user(&mut self, content: Value, message_id: Option<&str>) -> bool {
        let uuid = Uuid::new_v4().to_string();
        if let Some(id) = message_id {
            self.undelivered.insert(uuid.clone(), id.to_string());
        }
        let sent = self.send(
            json!({"type": "user", "uuid": uuid, "message": {"role": "user", "content": content}}),
        );
        if !sent {
            self.undelivered.remove(&uuid);
        }
        sent
    }

    fn allow(&mut self, id: &str, input: Value) -> bool {
        self.respond(id, json!({"behavior": "allow", "updatedInput": input}))
    }

    fn deny(&mut self, id: &str, message: &str) -> bool {
        self.respond(id, json!({"behavior": "deny", "message": message}))
    }

    fn respond(&mut self, id: &str, response: Value) -> bool {
        self.send(json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": id, "response": response
        }}))
    }

    fn error(&mut self, id: &str, error: &str) -> bool {
        self.send(json!({"type": "control_response", "response": {
            "subtype": "error", "request_id": id, "error": error
        }}))
    }

    pub fn ingest(&mut self, message: &Value) -> Option<TurnStreamEvent> {
        let kind = message["type"].as_str().unwrap_or_default();
        if matches!(kind, "assistant" | "stream_event" | "user")
            && message["parent_tool_use_id"].is_null()
        {
            self.idle = false;
        }
        match kind {
            "system" if message["subtype"] == "init" => {
                if self.initialized {
                    self.log("duplicate system/init");
                    return None;
                }
                self.initialized = true;
                // This is the installed CLI's advertised control capability; older
                // builds have no capability list. Do not infer support from a version.
                self.enabled = self.stdin.is_some()
                    && message["capabilities"]
                        .as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == "interrupt_receipt_v1"));
                if !self.enabled {
                    self.one_shot();
                }
            }
            "system" if message["subtype"] == "background_tasks_changed" => {
                match message["tasks"].as_array() {
                    Some(tasks) => {
                        self.tasks = tasks.iter().filter(|task| task["ambient"] != true).count()
                    }
                    None => self.log("background_tasks_changed without tasks"),
                }
            }
            "result" => {
                for uuid in message["user_message_uuids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    self.undelivered.remove(uuid);
                }
                // Background work and queued messages each bring another result.
                let inputs_done = message["queued_turn_count"].as_u64().unwrap_or(0) == 0
                    && self.undelivered.is_empty();
                if !self.enabled || self.stopping || (self.tasks == 0 && inputs_done) {
                    if !self.request_context_usage() {
                        self.end();
                    }
                } else {
                    // Queued input starts another model turn that Stop must interrupt.
                    self.idle = inputs_done;
                }
            }
            "control_request" => return self.route(message),
            "control_response" => {
                let response = &message["response"];
                if response["request_id"].as_str().is_some_and(|id| {
                    self.context_request
                        .as_ref()
                        .is_some_and(|(pending, _)| pending == id)
                }) {
                    // Failure is silent: the result's context window stands.
                    let usage = &response["response"];
                    if let (Some(used), Some(window)) =
                        (usage["totalTokens"].as_u64(), usage["maxTokens"].as_u64())
                    {
                        self.context = Some(json!({"used_tokens": used, "window_tokens": window}));
                    }
                    self.context_request = None;
                    self.end();
                } else if self.interrupt.is_some()
                    && self.interrupt.as_deref() == response["request_id"].as_str()
                    && response["subtype"] == "success"
                    && response["response"]["still_queued"]
                        .as_array()
                        .is_some_and(|ids| ids.is_empty())
                {
                    self.interrupt.take();
                    self.interrupted = true;
                } else {
                    self.log("unexpected or malformed control response");
                }
            }
            "control_cancel_request" => {
                if let Some(id) = message["request_id"]
                    .as_str()
                    .filter(|id| self.seen.contains(*id))
                {
                    self.pending.remove(id);
                } else {
                    self.log("unknown cancellation request");
                }
            }
            kind if kind.starts_with("control_") => self.log("unknown control message"),
            _ => {}
        }
        None
    }

    /// Answers every control request that names an id, whatever its shape.
    fn route(&mut self, message: &Value) -> Option<TurnStreamEvent> {
        let Some(id) = message["request_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
        else {
            self.log("control request missing request_id");
            return None;
        };
        // Answer whenever stdin is open, even before init; only interrupt
        // waits for the advertised capability.
        if self.stdin.is_none() {
            self.log("control request after stdin closed");
            return None;
        }
        if !self.seen.insert(id.to_string()) {
            self.log("duplicate control request");
            // A pending question still owes its one answer.
            if !self.pending.contains_key(id) {
                self.error(id, "duplicate control request id");
            }
            return None;
        }
        let request = &message["request"];
        if request["subtype"] != "can_use_tool" {
            let subtype = request["subtype"].as_str().unwrap_or("(missing)");
            self.error(
                id,
                &format!("unsupported control request subtype: {subtype}"),
            );
            return None;
        }
        let input = &request["input"];
        let Some(tool) = request["tool_name"]
            .as_str()
            .and_then(non_empty)
            .filter(|_| input.is_object())
        else {
            self.log("malformed permission request");
            self.error(id, "can_use_tool needs a tool_name and an object input");
            return None;
        };
        if tool == "ExitPlanMode" && self.plan_mode {
            let Some(plan) = input["plan"].as_str().and_then(non_empty) else {
                self.deny(id, PLAN_MISSING);
                return None;
            };
            let plan = plan.to_string();
            self.deny(id, PLAN_SUBMITTED);
            // The plan Codex reports as a completed Plan item; one item per
            // turn, so a resubmitted plan replaces the earlier one.
            return Some(stream_event(
                TurnStreamEventKind::ContentCompleted,
                message["session_id"].as_str(),
                "plan_completed",
                |event| {
                    event.channel = Some(TurnStreamChannel::Plan);
                    event.content_delta = Some(plan.clone());
                    event.message = Some(plan);
                    event.source.item_id = Some("plan".into());
                },
            ));
        }
        if tool != "AskUserQuestion" {
            self.allow(id, input.clone());
            return None;
        }
        if matches!(self.questions, Questions::Deny) {
            self.deny(id, NO_QUESTION_HANDLER);
            return None;
        }
        let questions = match normalized_questions(input) {
            Ok(questions) => questions,
            Err(reason) => {
                self.log(reason);
                self.deny(id, &format!("{reason}. {NO_QUESTION_HANDLER}"));
                return None;
            }
        };
        if let Questions::Flow(handler) = &self.questions {
            match handler(json!({"questions": questions, "agent_request_id": id})) {
                Ok(poll) => self.clarifications.push((id.to_string(), poll)),
                Err(reason) => {
                    self.log(&reason);
                    self.deny(id, &format!("{reason}. {NO_QUESTION_HANDLER}"));
                    return None;
                }
            }
        }
        self.pending.insert(id.to_string(), input.clone());
        Some(stream_event(
            TurnStreamEventKind::RequestUserInputRequested,
            message["session_id"].as_str(),
            "request_user_input_requested",
            |event| {
                event.source.item_id = Some(id.to_string());
                event.request_user_input = Some(json!({"request_id": id, "questions": questions}));
            },
        ))
    }
}

impl Control {
    /// Delivers answered flow clarifications. Err once the run stops waiting
    /// for them, such as on cancel.
    pub fn poll_clarifications(&mut self) -> Result<(), String> {
        let mut index = 0;
        while index < self.clarifications.len() {
            match (self.clarifications[index].1)()? {
                Some(answers) => {
                    let (id, _poll) = self.clarifications.remove(index);
                    self.deliver(&id, &answers);
                }
                None => index += 1,
            }
        }
        Ok(())
    }

    /// Allows a pending question with `updatedInput.answers`, keyed by the
    /// `question-N` ids its normalized questions carry.
    fn deliver(&mut self, id: &str, answers: &BTreeMap<String, String>) -> bool {
        let Some(mut input) = self.pending.get(id).cloned() else {
            return false;
        };
        let questions = input["questions"].as_array().unwrap();
        let mut by_prompt = serde_json::Map::new();
        for (index, question) in questions.iter().enumerate() {
            let Some(answer) = answers
                .get(&format!("question-{}", index + 1))
                .and_then(|answer| non_empty(answer))
            else {
                return false;
            };
            by_prompt.insert(
                question["question"].as_str().unwrap().to_string(),
                json!(answer),
            );
        }
        if by_prompt.len() != answers.len() {
            return false;
        }
        input["answers"] = Value::Object(by_prompt);
        self.pending.remove(id);
        self.allow(id, input)
    }
}

fn normalized_questions(input: &Value) -> Result<Vec<Value>, &'static str> {
    let questions = input["questions"]
        .as_array()
        .filter(|q| !q.is_empty())
        .ok_or("question request has no questions")?;
    let mut normalized = Vec::new();
    let mut prompts = BTreeSet::new();
    for (index, question) in questions.iter().enumerate() {
        let prompt = question["question"]
            .as_str()
            .and_then(non_empty)
            .ok_or("malformed question")?;
        if !prompts.insert(prompt)
            || !question["options"].as_array().is_some_and(|options| {
                options
                    .iter()
                    .all(|option| option["label"].as_str().and_then(non_empty).is_some())
            })
        {
            return Err("malformed question options or duplicate prompt");
        }
        let mut question = question.clone();
        question["id"] = json!(format!("question-{}", index + 1));
        question["allow_other"] = json!(true);
        // ponytail: existing segments store one string per question.
        // Multi-select remains answerable as comma-separated free text.
        if question["multiSelect"] == true {
            question["question_type"] = json!("FREEFORM");
            let labels = question["options"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|option| option["label"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            question["question"] = json!(format!(
                "{prompt} (Choose one or more: {labels}; separate answers with commas.)"
            ));
        }
        normalized.push(question);
    }
    Ok(normalized)
}

fn find(project_path: &str, conversation_id: &str) -> Option<Arc<Mutex<Control>>> {
    registry()
        .lock()
        .unwrap()
        .get(&(project_path.into(), conversation_id.into()))
        .and_then(Weak::upgrade)
}

pub(super) fn interrupt(project_path: &str, conversation_id: &str) -> bool {
    let Some(control) = find(project_path, conversation_id) else {
        return false;
    };
    let mut control = control.lock().unwrap();
    if !control.enabled || !matches!(control.child.try_wait(), Ok(None)) {
        return false;
    }
    // Only background tasks are left: no model turn to interrupt.
    if control.idle {
        control.end();
        return true;
    }
    if control.stopping {
        return true;
    }
    let id = Uuid::new_v4().to_string();
    control.interrupt = Some(id.clone());
    control.stopping = true;
    control.send(
        json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}}),
    )
}

pub(super) fn send_message(
    project_path: &str,
    conversation_id: &str,
    message_id: &str,
    message: &str,
) -> bool {
    let Some(control) = find(project_path, conversation_id) else {
        return false;
    };
    let mut control = control.lock().unwrap();
    // Before init the CLI's control support is unknown, and a stopping turn
    // would drop the message.
    if !control.enabled || control.stopping || !matches!(control.child.try_wait(), Ok(None)) {
        return false;
    }
    // The message starts model work that Stop must interrupt.
    control.idle = false;
    let sent = control.send_user(json!(message), Some(message_id));
    if sent {
        control
            .sent
            .push((message_id.to_string(), message.to_string()));
    }
    sent
}

pub(super) fn terminate(project_path: &str, conversation_id: &str) -> bool {
    let Some(control) = find(project_path, conversation_id) else {
        return false;
    };
    let mut control = control.lock().unwrap();
    if control.ended || !matches!(control.child.try_wait(), Ok(None)) {
        return false;
    }
    control.stopping = true;
    control.end();
    let pid = control.child.id();
    crate::local_environment::terminate_process_group(pid, Some(&mut control.child));
    true
}

pub(super) fn answer(request: AgentRequestUserInputAnswerRequest) -> AgentTurnOutput {
    let delivered = find(&request.project_path, &request.conversation_id).is_some_and(|control| {
        let mut control = control.lock().unwrap();
        if control.stdin.is_none() || !matches!(control.child.try_wait(), Ok(None)) {
            return false;
        }
        // Flow clarifications are answered through the run, not the chat route.
        if matches!(control.questions, Questions::Flow(_)) {
            return false;
        }
        if request.request_user_input.as_ref().is_some_and(|payload| {
            payload["status"] == "expired"
                || payload["request_id"]
                    .as_str()
                    .is_some_and(|id| id != request.request_id)
        }) {
            return false;
        }
        control.deliver(&request.request_id, &request.answers)
    });
    if delivered {
        AgentTurnOutput {
            events: vec![stream_event(
                TurnStreamEventKind::Other("request_user_input_answer_delivered".into()),
                None,
                "request_user_input_answer_delivered",
                |event| {
                    event.status = Some("delivered".into());
                },
            )],
            ..AgentTurnOutput::default()
        }
    } else {
        AgentTurnOutput { thread_resume_failure: Some(AgentThreadResumeFailure {
            message: "request-user-input answer could not resume because the live request is no longer pending.".into(),
            error_code: Some("request_user_input_not_pending".into()),
            details: Some(json!({"request_id": request.request_id})),
        }), ..AgentTurnOutput::default() }
    }
}
