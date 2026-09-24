//! Per-turn stdio control state. No process or pending question survives a turn.
use super::*;
use std::collections::BTreeSet;
use std::process::{Child, ChildStdin};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::agent::AgentRequestUserInputAnswerRequest;

type Key = (String, String);
type Registry = Mutex<BTreeMap<Key, Weak<Mutex<Control>>>>;
fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

pub(super) struct Control {
    pub child: Child,
    stdin: Option<ChildStdin>,
    enabled: bool,
    initialized: bool,
    pending: BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    interrupt: Option<String>,
    interrupted: bool,
    pub logs: Vec<AgentRawLogLine>,
}

impl Control {
    pub fn register(
        child: Child,
        stdin: ChildStdin,
        request: &AgentTurnRequest,
    ) -> Arc<Mutex<Self>> {
        let control = Arc::new(Mutex::new(Self {
            child,
            stdin: Some(stdin),
            enabled: false,
            initialized: false,
            pending: BTreeMap::new(),
            seen: BTreeSet::new(),
            interrupt: None,
            interrupted: false,
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

    pub fn close(&mut self) {
        self.enabled = false;
        self.stdin.take();
        self.pending.clear();
    }

    pub fn degrade(&mut self, reason: &str) {
        tracing::warn!(reason, "Claude Code control disabled");
        self.logs.push(AgentRawLogLine {
            direction: "control".into(),
            line: format!("Claude Code control disabled: {reason}"),
        });
        self.close();
    }

    fn send(&mut self, message: Value) -> bool {
        let line = message.to_string();
        let result = self
            .stdin
            .as_mut()
            .map(|stdin| writeln!(stdin, "{line}").and_then(|()| stdin.flush()));
        if !matches!(result, Some(Ok(()))) {
            self.degrade("stdin write failed");
            return false;
        }
        self.logs.push(AgentRawLogLine {
            direction: "stdin".into(),
            line,
        });
        true
    }

    fn allow(&mut self, id: &str, input: Value) -> bool {
        self.send(json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": id,
            "response": {"behavior": "allow", "updatedInput": input}
        }}))
    }

    pub fn ingest(&mut self, message: &Value) -> Option<TurnStreamEvent> {
        if message["type"] == "system" && message["subtype"] == "init" {
            if self.initialized {
                self.degrade("duplicate system/init");
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
                self.close();
            }
            return None;
        }
        if message["type"] == "result" {
            self.close();
            return None;
        }
        let kind = message["type"].as_str().unwrap_or_default();
        if !kind.starts_with("control_") {
            return None;
        }
        if !self.enabled {
            self.degrade("unexpected control traffic while disabled");
            return None;
        }
        match kind {
            "control_request" => {
                let Some(id) = message["request_id"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())
                else {
                    self.degrade("control request missing request_id");
                    return None;
                };
                let request = &message["request"];
                if !self.seen.insert(id.to_string())
                    || request["subtype"] != "can_use_tool"
                    || request["tool_name"].as_str().and_then(non_empty).is_none()
                    || !request["input"].is_object()
                {
                    self.degrade("duplicate or malformed permission request");
                    return None;
                }
                if request["tool_name"] != "AskUserQuestion" {
                    self.allow(id, request["input"].clone());
                    return None;
                }
                let Some(questions) = request["input"]["questions"]
                    .as_array()
                    .filter(|q| !q.is_empty())
                else {
                    self.degrade("question request has no questions");
                    return None;
                };
                let mut normalized = Vec::new();
                let mut prompts = BTreeSet::new();
                for (index, question) in questions.iter().enumerate() {
                    let Some(prompt) = question["question"].as_str().and_then(non_empty) else {
                        self.degrade("malformed question");
                        return None;
                    };
                    if !prompts.insert(prompt)
                        || !question["options"].as_array().is_some_and(|options| {
                            options.iter().all(|option| {
                                option["label"].as_str().and_then(non_empty).is_some()
                            })
                        })
                    {
                        self.degrade("malformed question options or duplicate prompt");
                        return None;
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
                        question["question"] = json!(format!("{prompt} (Choose one or more: {labels}; separate answers with commas.)"));
                    }
                    normalized.push(question);
                }
                self.pending
                    .insert(id.to_string(), request["input"].clone());
                Some(stream_event(
                    TurnStreamEventKind::RequestUserInputRequested,
                    message["session_id"].as_str(),
                    "request_user_input_requested",
                    |event| {
                        event.source.item_id = Some(id.to_string());
                        event.request_user_input =
                            Some(json!({"request_id": id, "questions": normalized}));
                    },
                ))
            }
            "control_response" => {
                let response = &message["response"];
                if self.interrupt.as_deref() != response["request_id"].as_str()
                    || self.interrupt.is_none()
                    || response["subtype"] != "success"
                    || !response["response"]["still_queued"]
                        .as_array()
                        .is_some_and(|ids| ids.is_empty())
                {
                    self.degrade("unexpected or malformed interrupt receipt");
                } else {
                    self.interrupt.take();
                    self.interrupted = true;
                }
                None
            }
            "control_cancel_request" => {
                if let Some(id) = message["request_id"]
                    .as_str()
                    .filter(|id| self.seen.contains(*id))
                {
                    self.pending.remove(id);
                } else {
                    self.degrade("unknown cancellation request");
                }
                None
            }
            _ => {
                self.degrade("unknown control message");
                None
            }
        }
    }
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
    if control.interrupt.is_some() {
        return true;
    }
    let id = Uuid::new_v4().to_string();
    control.interrupt = Some(id.clone());
    control.send(
        json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}}),
    )
}

pub(super) fn answer(request: AgentRequestUserInputAnswerRequest) -> AgentTurnOutput {
    let delivered = find(&request.project_path, &request.conversation_id).is_some_and(|control| {
        let mut control = control.lock().unwrap();
        if !control.enabled || !matches!(control.child.try_wait(), Ok(None)) {
            control.close();
            return false;
        }
        let Some(mut input) = control.pending.get(&request.request_id).cloned() else {
            return false;
        };
        if request.request_user_input.as_ref().is_some_and(|payload| {
            payload["status"] == "expired"
                || payload["request_id"]
                    .as_str()
                    .is_some_and(|id| id != request.request_id)
        }) {
            return false;
        }
        let questions = input["questions"].as_array().unwrap();
        let mut answers = serde_json::Map::new();
        for (index, question) in questions.iter().enumerate() {
            let Some(answer) = request
                .answers
                .get(&format!("question-{}", index + 1))
                .and_then(|answer| non_empty(answer))
            else {
                return false;
            };
            answers.insert(
                question["question"].as_str().unwrap().to_string(),
                json!(answer),
            );
        }
        if answers.len() != request.answers.len() {
            return false;
        }
        input["answers"] = Value::Object(answers);
        control.pending.remove(&request.request_id);
        control.allow(&request.request_id, input)
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
