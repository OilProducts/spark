//! Per-turn stdio control state, owned by the turn's connection worker. No
//! process or pending question survives a turn.
use super::*;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::agent::AgentRequestUserInputAnswerRequest;
use crate::codex_app_server::json_len;
use crate::stdio_transport::{
    self, sleep_until, Blocking, Budget, Connection, End, Endpoints, Held, Incoming, Outgoing,
    Refused, Status, Stop, LIMITS, PENDING_LIMIT,
};

const NO_QUESTION_HANDLER: &str =
    "No one can answer questions during this turn. Proceed on your best judgment and state the assumptions you made.";
const PLAN_SUBMITTED: &str =
    "Your plan was submitted to the user for review. Stop here; do not implement it.";
const PLAN_MISSING: &str = "ExitPlanMode needs your plan in `plan`.";
const CONTEXT_USAGE_TIMEOUT: Duration = Duration::from_secs(5);
const INIT_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a flow question's answer is looked for.
const FLOW_POLL: Duration = Duration::from_millis(100);
/// How long an exited process's stderr may take to end, for its tail.
const STDERR_WAIT: Duration = Duration::from_secs(1);

type Key = (String, String);
type Registry = Mutex<BTreeMap<Key, Weak<Live>>>;
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

/// A running turn as other threads reach it: commands its worker answers,
/// and a stop that goes past the worker straight to the process group.
pub(super) struct Live {
    commands: UnboundedSender<(Command, Held)>,
    budget: Budget,
    stop: Stop,
    /// The turn has ended or its process has exited.
    closed: Arc<AtomicBool>,
}

type Reply = std::sync::mpsc::Sender<bool>;

/// What another thread asks of a running turn; each is answered once.
enum Command {
    Interrupt(Reply),
    Send {
        message_id: String,
        message: String,
        reply: Reply,
    },
    Answer {
        request_id: String,
        answers: BTreeMap<String, String>,
        reply: Reply,
    },
}

impl Command {
    fn refuse(self) {
        let (Self::Interrupt(reply) | Self::Send { reply, .. } | Self::Answer { reply, .. }) = self;
        let _ = reply.send(false);
    }
}

/// What a turn's worker hands its caller, in order: events for the sink,
/// then the turn itself.
pub(super) enum Message {
    Event(TurnStreamEvent),
    /// With the events sequenced at teardown, which reach the sink after
    /// everything queued.
    Done(Box<Turn>, Outcome, Vec<TurnStreamEvent>),
}

/// How one process's part of the turn ended.
pub(super) enum Outcome {
    Ended,
    /// The turn failed and is not to be resumed.
    Failed(String),
    /// The process exited before the turn ended, with its exit status.
    Exited(String),
}

/// Flow clarification work finished off the runtime.
enum Completion {
    /// The clarification handler took a question.
    Asked {
        id: String,
        poll: Result<crate::codergen::ClarificationPoll, String>,
    },
    Polled {
        id: String,
        poll: crate::codergen::ClarificationPoll,
        answers: Result<Option<BTreeMap<String, String>>, String>,
    },
}

/// A flow question awaiting its answer.
struct Clarification {
    id: String,
    /// Its event, sequenced once the handler has taken the question.
    event: Option<TurnStreamEvent>,
    /// Absent while the handler or a poll runs.
    poll: Option<crate::codergen::ClarificationPoll>,
}

/// Starts the turn's process with its worker, which sends `content` as the
/// prompt and hands back events for the sink (if `sink`), then the turn. The
/// turn stays with the caller if the process cannot start.
pub(super) fn start(
    command: std::process::Command,
    request: &AgentTurnRequest,
    questions: Questions,
    content: Value,
    turn: &mut Turn,
    sink: bool,
) -> std::io::Result<(Connection, Arc<Live>, std::sync::mpsc::Receiver<Message>)> {
    let (commands, received) = unbounded_channel();
    let (completions, completed) = unbounded_channel();
    let (messages, delivered) = std::sync::mpsc::channel();
    let (hand_over, handed) = tokio::sync::oneshot::channel::<Turn>();
    let closed = Arc::new(AtomicBool::new(false));
    let work = match questions {
        Questions::Flow(_) => Some(Blocking::spawn(&LIMITS)?),
        _ => None,
    };
    let worker = Worker {
        control: Control::new(request, questions, work, completions),
        turn: Turn::default(),
        messages,
        sink,
        late: Vec::new(),
        closed: Arc::clone(&closed),
        exited: false,
        initialized: false,
    };
    let connection = stdio_transport::launch(command, LIMITS, move |endpoints| async move {
        if let Ok(turn) = handed.await {
            worker
                .run(endpoints, turn, content, received, completed)
                .await;
        }
    })?;
    if let Err(unstarted) = hand_over.send(std::mem::take(turn)) {
        *turn = unstarted;
    }
    let live = Arc::new(Live {
        commands,
        budget: Budget::new(&LIMITS),
        stop: connection.stop_handle(),
        closed,
    });
    let mut entries = registry().lock().unwrap();
    entries.retain(|_, entry| entry.strong_count() > 0);
    entries.insert(
        (
            request.project_path.clone(),
            request.conversation_id.clone(),
        ),
        Arc::downgrade(&live),
    );
    drop(entries);
    Ok((connection, live, delivered))
}

struct Control {
    stdin: Option<Outgoing>,
    questions: Questions,
    /// A plan-mode turn: `ExitPlanMode` submits the turn's plan.
    plan_mode: bool,
    /// Flow clarifications awaiting answers.
    clarifications: Vec<Clarification>,
    /// Runs the flow's clarification handler and its polls in order.
    work: Option<Blocking>,
    completions: UnboundedSender<Completion>,
    /// When flow answers are next looked for.
    flow_due: Option<Instant>,
    enabled: bool,
    initialized: bool,
    pending: BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    interrupt: Option<String>,
    interrupted: bool,
    /// Stop was requested; the next result ends the turn.
    stopping: bool,
    /// Live non-ambient background tasks, from the last `background_tasks_changed`.
    tasks: usize,
    /// This process's prompt, by uuid. Unlike a mid-turn message it has no
    /// caller message id, so it never reports delivery.
    prompt: Option<String>,
    /// The CLI queued the prompt behind a command of its own and has not
    /// started it yet.
    prompt_queued: bool,
    /// Messages sent after the prompt whose uuid no result has confirmed
    /// yet, by uuid, with their caller's message id.
    undelivered: BTreeMap<String, String>,
    /// A result left only background tasks running and no main model turn
    /// has started since.
    idle: bool,
    /// The `get_context_usage` request sent after the final result, with
    /// when it gives up; the turn ends at its answer.
    context_request: Option<(String, Instant)>,
    /// `{used_tokens, window_tokens}` from the answered request.
    context: Option<Value>,
    ended: bool,
    /// Why the turn failed; a failed turn is not resumed.
    failure: Option<String>,
    /// Why stdin broke: nothing more reaches the process.
    broken: Option<String>,
    logs: Vec<AgentRawLogLine>,
}

impl Control {
    fn new(
        request: &AgentTurnRequest,
        questions: Questions,
        work: Option<Blocking>,
        completions: UnboundedSender<Completion>,
    ) -> Self {
        Self {
            stdin: None,
            questions,
            plan_mode: plan_mode(request),
            clarifications: Vec::new(),
            work,
            completions,
            flow_due: None,
            enabled: false,
            initialized: false,
            pending: BTreeMap::new(),
            seen: BTreeSet::new(),
            interrupt: None,
            interrupted: false,
            stopping: false,
            tasks: 0,
            prompt: None,
            prompt_queued: false,
            undelivered: BTreeMap::new(),
            idle: false,
            context_request: None,
            context: None,
            ended: false,
            failure: None,
            broken: None,
            logs: Vec::new(),
        }
    }

    fn interrupted_result(&self, message: &Value) -> bool {
        self.interrupted
            && message["type"] == "result"
            && matches!(
                message["terminal_reason"].as_str(),
                Some("aborted_streaming" | "aborted_tools")
            )
    }

    /// A successful result while the prompt waits behind another command,
    /// such as a resumed session's own task notification, answers that
    /// command and not this turn. A result naming the prompt is its own; a
    /// stopping turn ends at any result.
    fn foreign_result(&self, message: &Value) -> bool {
        self.prompt_queued
            && !self.stopping
            && message["type"] == "result"
            && message["is_error"] != true
            && !message["user_message_uuids"]
                .as_array()
                .is_some_and(|uuids| {
                    uuids
                        .iter()
                        .any(|uuid| uuid.as_str() == self.prompt.as_deref())
                })
    }

    /// The final result has arrived; the turn ends once the context usage
    /// request is answered or times out.
    fn finished(&self) -> bool {
        self.ended || self.context_request.is_some()
    }

    /// Ends a turn still waiting on its context usage once it is overdue or
    /// the process has exited. Silent: the result's context window stands.
    fn expire_context_request(&mut self, exited: bool) {
        if self
            .context_request
            .as_ref()
            .is_some_and(|(_, deadline)| exited || Instant::now() >= *deadline)
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
        self.context_request = Some((id, Instant::now() + CONTEXT_USAGE_TIMEOUT));
        true
    }

    /// Ends the turn: no more input, answers or interrupts reach the process.
    fn end(&mut self) {
        self.ended = true;
        self.one_shot();
    }

    /// Closes stdin, once the lines queued before it are written, so the CLI
    /// ends at its result, as it does without control. Pending questions are
    /// denied first: the CLI waits on every request.
    fn one_shot(&mut self) {
        self.enabled = false;
        self.clarifications.clear();
        for id in std::mem::take(&mut self.pending).into_keys() {
            if self.stdin.is_none() {
                break;
            }
            self.deny(
                &id,
                "No one can answer this question now; proceed with your best judgment.",
            );
        }
        self.stdin.take();
    }

    /// Drops every question and stdin without writing: the process can no
    /// longer read them.
    fn abandon(&mut self) {
        self.stdin = None;
        self.pending.clear();
        self.one_shot();
    }

    fn log(&mut self, reason: &str) {
        tracing::warn!(reason, "Claude Code control traffic");
        self.logs.push(AgentRawLogLine {
            direction: "control".into(),
            line: format!("Claude Code control: {reason}"),
        });
    }

    /// The first failure ends the turn; later ones only log.
    fn fail(&mut self, reason: String) {
        self.log(&reason);
        self.failure.get_or_insert(reason);
    }

    fn send(&mut self, message: Value) -> bool {
        let line = message.to_string();
        let refused = match self.stdin.as_ref().map(|stdin| stdin.send(line.clone())) {
            // Admitted: written in order behind earlier lines. A write that
            // fails later is reported through the connection's status.
            Some(Ok(_written)) => {
                self.logs.push(AgentRawLogLine {
                    direction: "stdin".into(),
                    line,
                });
                return true;
            }
            Some(Err(refused)) => refused,
            None => Refused::Closed,
        };
        match refused {
            // The writer stopped at a failed write, which the connection
            // status reports.
            Refused::Closed => {}
            Refused::Full => self.fail("claude code overloaded: its stdin queue is full".into()),
            Refused::TooLarge => self.fail(format!(
                "claude code input exceeds the {} MiB line limit",
                LIMITS.frame_bytes >> 20
            )),
        }
        // Nothing after it could reach the process in order.
        self.abandon();
        false
    }

    /// Writes a user message stamped with a uuid: the prompt, or a message
    /// after it, named by `message_id`, that holds the turn open until a
    /// result lists it as delivered.
    fn send_user(&mut self, content: Value, message_id: Option<&str>) -> bool {
        let uuid = Uuid::new_v4().to_string();
        let message =
            json!({"type": "user", "uuid": uuid, "message": {"role": "user", "content": content}});
        // A live message longer than one line is refused unwritten.
        if message_id.is_some() && json_len(&message) > LIMITS.frame_bytes {
            return false;
        }
        match message_id {
            Some(id) => {
                self.undelivered.insert(uuid.clone(), id.to_string());
            }
            None => self.prompt = Some(uuid.clone()),
        }
        let sent = self.send(message);
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

    fn ingest(&mut self, message: &Value) -> Option<TurnStreamEvent> {
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
            // Lifecycle frames carry their own uuid; command_uuid names the command.
            "command_lifecycle"
                if self.prompt.is_some()
                    && message["command_uuid"].as_str() == self.prompt.as_deref() =>
            {
                match message["state"].as_str() {
                    Some("queued") => self.prompt_queued = true,
                    Some("started") => self.prompt_queued = false,
                    _ => {}
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
                    // The prompt was processed, whether or not it was seen starting.
                    if self.prompt.as_deref() == Some(uuid) {
                        self.prompt_queued = false;
                    }
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
                    // still_queued names messages the CLI holds for a follow-up
                    // turn; each must be one we sent and is not yet delivered.
                    && response["response"]["still_queued"]
                        .as_array()
                        .is_some_and(|ids| {
                            ids.iter().all(|id| {
                                id.as_str().is_some_and(|id| self.undelivered.contains_key(id))
                            })
                        })
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
                    self.clarifications.retain(|question| question.id != id);
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
        if self.pending.len() >= PENDING_LIMIT {
            self.fail("claude code overloaded: its question list is full".into());
            return None;
        }
        let event = stream_event(
            TurnStreamEventKind::RequestUserInputRequested,
            message["session_id"].as_str(),
            "request_user_input_requested",
            |event| {
                event.source.item_id = Some(id.to_string());
                event.request_user_input =
                    Some(json!({"request_id": id, "questions": questions.clone()}));
            },
        );
        self.pending.insert(id.to_string(), input.clone());
        let (Questions::Flow(handler), Some(work)) = (&self.questions, &self.work) else {
            return Some(event);
        };
        // The handler records the question for the run, off the runtime; the
        // question is raised once it has.
        let payload = json!({"questions": questions, "agent_request_id": id});
        let (handler, completions, key) = (
            Arc::clone(handler),
            self.completions.clone(),
            id.to_string(),
        );
        let queued = work.run(json_len(&payload), move || {
            let poll = handler(payload);
            let _ = completions.send(Completion::Asked { id: key, poll });
        });
        if queued.is_err() {
            self.fail("claude code overloaded: its clarification queue is full".into());
            return None;
        }
        self.clarifications.push(Clarification {
            id: id.to_string(),
            event: Some(event),
            poll: None,
        });
        None
    }

    /// ponytail: a flow question's answer lives in the run record, which
    /// another process writes, so each is polled every 100 ms while it waits,
    /// at most one poll at a time. Removing this needs cross-process
    /// notification.
    fn poll_clarifications(&mut self, now: Instant) {
        if self.clarifications.is_empty() {
            self.flow_due = None;
            return;
        }
        if !self.flow_due.is_some_and(|due| now >= due) {
            return;
        }
        self.flow_due = Some(now + FLOW_POLL);
        let Some(work) = &self.work else {
            return;
        };
        let mut queued = Ok(());
        for question in &mut self.clarifications {
            let Some(mut poll) = question.poll.take() else {
                continue;
            };
            let (id, completions) = (question.id.clone(), self.completions.clone());
            queued = work.run(0, move || {
                let answers = crate::codergen::ClarificationReply::polled_answers(poll());
                let _ = completions.send(Completion::Polled { id, poll, answers });
            });
            if queued.is_err() {
                break;
            }
        }
        if queued.is_err() {
            self.fail("claude code overloaded: its clarification queue is full".into());
        }
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

/// One process's connection worker: the sole reader of its stdout and
/// writer of its stdin, which sequences the turn's events and answers the
/// CLI and other threads whether or not anyone is consuming them.
struct Worker {
    control: Control,
    turn: Turn,
    messages: std::sync::mpsc::Sender<Message>,
    /// The caller has a sink.
    sink: bool,
    /// Events sequenced at teardown, for the handover.
    late: Vec<TurnStreamEvent>,
    closed: Arc<AtomicBool>,
    exited: bool,
    /// system/init arrived, or the turn stopped waiting for it.
    initialized: bool,
}

impl Worker {
    async fn run(
        mut self,
        endpoints: Endpoints,
        turn: Turn,
        content: Value,
        mut commands: UnboundedReceiver<(Command, Held)>,
        mut completed: UnboundedReceiver<Completion>,
    ) {
        let Endpoints {
            mut stdout,
            stdin,
            mut status,
            stop,
        } = endpoints;
        self.turn = turn;
        self.control.stdin = Some(stdin);
        // The prompt travels over stdin so arbitrarily large composed prompts
        // never hit argv limits.
        self.control.send_user(content, None);
        let init_deadline = Instant::now() + INIT_TIMEOUT;
        while !self.control.ended && self.control.failure.is_none() && self.control.broken.is_none()
        {
            let deadline = [
                (!self.initialized).then_some(init_deadline),
                self.control
                    .context_request
                    .as_ref()
                    .map(|(_, deadline)| *deadline),
                self.control.flow_due,
            ]
            .into_iter()
            .flatten()
            .min();
            tokio::select! {
                Some((command, _held)) = commands.recv() => self.command(command),
                line = stdout.next() => match line {
                    Some(line) => self.line(line.text),
                    None => {
                        if let Some(End::Failed(error)) = stdout.end() {
                            let error = format!("claude code stdout read failed: {error}");
                            self.control.fail(error);
                        }
                        break;
                    }
                },
                Some(completion) = completed.recv() => self.completion(completion),
                Ok(()) = status.changed() => {
                    let status = status.borrow_and_update().clone();
                    self.status(&status);
                }
                () = sleep_until(deadline) => {}
            }
            // Checked every round: continuous output must not hide an exit,
            // a missing system/init or a deadline.
            self.control.expire_context_request(self.exited);
            if !self.initialized && Instant::now() >= init_deadline && !self.control.ended {
                self.control
                    .log("system/init timeout; using one-shot input");
                self.control.one_shot();
                self.initialized = true;
            }
            self.control.poll_clarifications(Instant::now());
        }

        // A stop whose end of output was read first still ends the turn.
        let latest = status.borrow_and_update().clone();
        self.status(&latest);
        // Output ended while the context usage was pending: the turn is done.
        self.control.expire_context_request(true);
        // Answered after the turn completed, so it rides the turn's output.
        if let (Some(context), Some(usage)) = (
            self.control.context.take(),
            self.turn.state.usage_payload.as_mut(),
        ) {
            usage["context"] = context;
        }
        self.turn.stopping |= self.control.stopping;
        let ended = self.control.ended;
        self.control.end();
        // No more commands: every caller still waiting hears false.
        self.closed.store(true, Ordering::SeqCst);
        commands.close();
        while let Ok((command, _held)) = commands.try_recv() {
            command.refuse();
        }
        // Messages the turn ended without using, such as on Stop.
        let not_delivered = std::mem::take(&mut self.control.undelivered)
            .into_values()
            .map(|message_id| {
                stream_event(
                    TurnStreamEventKind::Other("user_message_not_delivered".into()),
                    self.turn.state.session_id.as_deref(),
                    "user_message_not_delivered",
                    |event| {
                        event.source.item_id = Some(message_id);
                        event.message =
                            Some("Not sent: the turn ended before the agent read it.".into());
                    },
                )
            })
            .collect();
        self.settle(not_delivered);
        // Stopped while idle: the last result, so far commentary, is final.
        if ended && self.control.failure.is_none() && !self.turn.state.completed {
            if let Some(result) = self.turn.state.result_payload.clone() {
                let events = self.turn.state.ingest_result(&result, true);
                self.settle(events);
            }
        }
        self.turn.raw_log_lines.append(&mut self.control.logs);
        let (exit, by_itself) = reap(&mut stdout, &mut status, &stop).await;
        let outcome = if let Some(reason) = self.control.failure.take() {
            // Not resumed: the failure was not the process's.
            self.turn.stopping = true;
            Outcome::Failed(reason)
        } else if ended {
            Outcome::Ended
        } else if let Some(error) = self.control.broken.take().filter(|_| !by_itself) {
            // Alive with its stdin closed: not a crash, so not resumed.
            self.turn.stopping = true;
            Outcome::Failed(format!("claude code stdin write failed: {error}"))
        } else {
            // Exited, or stdin broke as it died: its exit decides, as for
            // any crash. Its last words, unless a descendant holds stderr open.
            let _ =
                tokio::time::timeout(STDERR_WAIT, status.wait_for(|status| status.stderr_closed))
                    .await;
            Outcome::Exited(exit)
        };
        let _ = self
            .messages
            .send(Message::Done(Box::new(self.turn), outcome, self.late));
    }

    /// Records events sequenced at teardown for the handover rather than
    /// the sink's queue.
    fn settle(&mut self, events: Vec<TurnStreamEvent>) {
        let (late, sink) = (&mut self.late, self.sink);
        self.turn.push(events, |event| {
            if sink {
                late.push(event.clone());
            }
        });
    }

    /// Sequences events: each goes to the sink's queue without waiting on
    /// the sink, and into the turn's record. A slow sink falls behind rather
    /// than failing the turn or stalling the worker it may be waiting on.
    fn push(&mut self, events: Vec<TurnStreamEvent>) {
        let Self {
            turn,
            sink,
            messages,
            ..
        } = self;
        // ponytail: unbounded, like the turn's record, which already holds
        // every event.
        turn.push(events, |event| {
            if *sink {
                let _ = messages.send(Message::Event(event.clone()));
            }
        });
    }

    fn line(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        self.turn.raw_log_lines.push(AgentRawLogLine {
            direction: "stdout".to_string(),
            line: line.clone(),
        });
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            self.control.log("malformed JSON on stdout");
            return;
        };
        if message["type"] == "system" && message["subtype"] == "init" {
            self.initialized = true;
        }
        // Kept in the raw log only: it neither ends the turn nor reports text
        // or usage.
        if self.control.foreign_result(&message) {
            self.control
                .log("result before the prompt started; waiting for the prompt's");
            return;
        }
        // 2.1.220 reports an interrupted stream as an error result. Only an
        // acknowledged interrupt with an explicit abort reason is normal
        // completion; genuine execution errors remain errors.
        if self.control.interrupted_result(&message) {
            message["is_error"] = json!(false);
            message["subtype"] = json!("success");
            let (source, text) =
                crate::events::latest_assistant_text(&self.turn.events).unwrap_or_default();
            let item_id = source.item_id;
            if item_id.is_some() {
                self.turn.state.last_text_item_id = item_id;
            }
            message["result"] = json!(non_empty(&text).unwrap_or("Stopped."));
        }
        let request_event = self.control.ingest(&message);
        let ended = self.control.finished();
        let mut normalized = self.turn.state.ingest(&message, ended);
        if let Some(mut event) = request_event {
            event.source.app_thread_id = self.turn.state.session_id.clone();
            normalized.push(event);
        }
        self.push(normalized);
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Interrupt(reply) => {
                let interrupted = self.interrupt();
                let _ = reply.send(interrupted);
            }
            Command::Send {
                message_id,
                message,
                reply,
            } => {
                let sent = self.send_message(message_id, message);
                let _ = reply.send(sent);
            }
            Command::Answer {
                request_id,
                answers,
                reply,
            } => {
                // Flow clarifications are answered through the run, not the chat route.
                let delivered = self.control.stdin.is_some()
                    && !self.exited
                    && !matches!(self.control.questions, Questions::Flow(_))
                    && self.control.deliver(&request_id, &answers);
                let _ = reply.send(delivered);
            }
        }
    }

    fn interrupt(&mut self) -> bool {
        let control = &mut self.control;
        if !control.enabled || self.exited {
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

    fn send_message(&mut self, message_id: String, message: String) -> bool {
        let control = &mut self.control;
        // Before init the CLI's control support is unknown, and a stopping turn
        // would drop the message.
        if !control.enabled || control.stopping || self.exited {
            return false;
        }
        if !control.send_user(json!(message), Some(&message_id)) {
            return false;
        }
        // The message starts model work that Stop must interrupt; a refused
        // one starts nothing.
        control.idle = false;
        // Sequenced as it is admitted, before any output read after it; the
        // reply never waits on the sink, which may be the caller.
        let delivered = self
            .turn
            .state
            .delivered_events(vec![(message_id, message)]);
        self.push(delivered);
        true
    }

    fn completion(&mut self, completion: Completion) {
        let control = &mut self.control;
        match completion {
            Completion::Asked { id, poll } => {
                // A question that expired meanwhile stays expired.
                let Some(index) = control.clarifications.iter().position(|q| q.id == id) else {
                    return;
                };
                match poll {
                    Ok(poll) => {
                        let question = &mut control.clarifications[index];
                        question.poll = Some(poll);
                        let event = question.event.take();
                        control.flow_due.get_or_insert_with(Instant::now);
                        if let Some(mut event) = event {
                            event.source.app_thread_id = self.turn.state.session_id.clone();
                            self.push(vec![event]);
                        }
                    }
                    Err(reason) => {
                        control.clarifications.remove(index);
                        control.pending.remove(&id);
                        control.log(&reason);
                        control.deny(&id, &format!("{reason}. {NO_QUESTION_HANDLER}"));
                    }
                }
            }
            Completion::Polled { id, poll, answers } => {
                let Some(index) = control.clarifications.iter().position(|q| q.id == id) else {
                    return;
                };
                match answers {
                    Ok(Some(answers)) => {
                        control.clarifications.remove(index);
                        control.deliver(&id, &answers);
                    }
                    Ok(None) => control.clarifications[index].poll = Some(poll),
                    // The run stopped waiting for an answer; do not resume the session.
                    Err(reason) => control.fail(reason),
                }
            }
        }
    }

    fn status(&mut self, status: &Status) {
        let control = &mut self.control;
        // A forced stop: the process group is going, so nothing more is
        // written, queued replies included.
        if status.stop_requested && !control.ended {
            control.stopping = true;
            control.ended = true;
            control.abandon();
            self.closed.store(true, Ordering::SeqCst);
        }
        // Process death expires questions at once, though a descendant may
        // hold stdout open; a late answer cannot revive them.
        if status.exited.is_some() && !self.exited {
            self.exited = true;
            self.closed.store(true, Ordering::SeqCst);
            control.pending.clear();
            control.clarifications.clear();
        }
        // A failed write: nothing more reaches the process. A turn whose
        // result is in keeps it, losing only its context usage; any other
        // ends here, settling its unused messages.
        if let Some(error) = status
            .write_failure
            .as_ref()
            .filter(|_| control.broken.is_none())
        {
            control.broken = Some(error.clone());
            control.abandon();
            control.expire_context_request(true);
            if !control.ended {
                control.log(&format!("stdin write failed: {error}"));
            }
        }
    }
}

/// Waits for the ended process to exit on its own for `EXIT_GRACE`, then
/// stops its group; its exit status, and whether it exited by itself.
/// Output it writes meanwhile is dropped.
async fn reap(
    stdout: &mut Incoming,
    status: &mut tokio::sync::watch::Receiver<Status>,
    stop: &Stop,
) -> (String, bool) {
    let grace = Instant::now() + EXIT_GRACE;
    let mut stopped = false;
    loop {
        match status.borrow_and_update().exited.clone() {
            Some(Ok(exit)) => return (exit.to_string(), !stopped),
            Some(Err(error)) => return (format!("wait failed: {error}"), !stopped),
            None => {}
        }
        tokio::select! {
            changed = status.changed() => {
                if changed.is_err() {
                    return ("unknown status".to_string(), !stopped);
                }
            }
            Some(_) = stdout.next(), if stdout.end().is_none() => {}
            () = sleep_until(Some(grace)), if !stopped => {
                stop.stop();
                stopped = true;
            }
        }
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

fn find(project_path: &str, conversation_id: &str) -> Option<Arc<Live>> {
    registry()
        .lock()
        .unwrap()
        .get(&(project_path.into(), conversation_id.into()))
        .and_then(Weak::upgrade)
}

/// Asks the conversation's running turn, unless its command queue is full;
/// false once it has ended.
fn ask(
    project_path: &str,
    conversation_id: &str,
    bytes: usize,
    command: impl FnOnce(Reply) -> Command,
) -> bool {
    let Some(live) = find(project_path, conversation_id) else {
        return false;
    };
    let Some(held) = live.budget.try_admit(bytes) else {
        return false;
    };
    let (reply, answered) = std::sync::mpsc::channel();
    live.commands.send((command(reply), held)).is_ok() && answered.recv().unwrap_or(false)
}

pub(super) fn interrupt(project_path: &str, conversation_id: &str) -> bool {
    ask(project_path, conversation_id, 0, Command::Interrupt)
}

pub(super) fn send_message(
    project_path: &str,
    conversation_id: &str,
    message_id: &str,
    message: &str,
) -> bool {
    // Longer than one line: refused before it is queued.
    if message.len() > LIMITS.frame_bytes {
        return false;
    }
    ask(
        project_path,
        conversation_id,
        message_id.len() + message.len(),
        |reply| Command::Send {
            message_id: message_id.to_string(),
            message: message.to_string(),
            reply,
        },
    )
}

/// Stops the process group directly; its worker sees the stop and ends the turn.
pub(super) fn terminate(project_path: &str, conversation_id: &str) -> bool {
    let Some(live) = find(project_path, conversation_id) else {
        return false;
    };
    if live.closed.load(Ordering::SeqCst) {
        return false;
    }
    live.stop.stop();
    true
}

pub(super) fn answer(request: AgentRequestUserInputAnswerRequest) -> AgentTurnOutput {
    let current = !request.request_user_input.as_ref().is_some_and(|payload| {
        payload["status"] == "expired"
            || payload["request_id"]
                .as_str()
                .is_some_and(|id| id != request.request_id)
    });
    let delivered = current
        && ask(
            &request.project_path,
            &request.conversation_id,
            json_len(&request.answers),
            |reply| Command::Answer {
                request_id: request.request_id.clone(),
                answers: request.answers.clone(),
                reply,
            },
        );
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
