//! Every conversation's inbox: chats own the runs they launch and hear back
//! from them, and any conversation messages any other by handle.
use super::mission_contracts::{project, wait_for, GatedAgent};
use super::review_artifact_contracts::{
    seed_conversation, settings, simple_flow, write_native_execution_profile,
};
use super::role_contracts::seed_bundled_roles;
use attractor_core::{RunRecord, RunResult};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
    AGENT_INSTRUCTIONS_METADATA_KEY,
};
use spark_common::settings::SparkSettings;
use spark_storage::{ConversationRepository, ProjectRegistry};
use spark_workspace::inbox::{install_runtime, send_message, InboxRuntime, ThreadMessageRequest};
use spark_workspace::{ConversationTurnRequest, RunLaunchRequest, WorkspaceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A gated agent whose running turn takes live messages while `live` is set.
#[derive(Default)]
struct LiveAgent {
    gated: GatedAgent,
    live: AtomicBool,
    taken: Mutex<Vec<String>>,
}
impl AgentTurnBackend for LiveAgent {
    fn send_message(&self, _: &str, _: &str, _: &str, message: &str) -> bool {
        let live = self.live.load(Ordering::SeqCst);
        if live {
            self.taken.lock().unwrap().push(message.to_string());
        }
        live
    }
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.gated.run_turn(request)
    }
}

struct Harness {
    temp: tempfile::TempDir,
    settings: SparkSettings,
    project: String,
    agent: Arc<LiveAgent>,
}
impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(&temp.path().canonicalize().unwrap());
        let project = project(&settings);
        write_native_execution_profile(&settings);
        seed_bundled_roles(&settings);
        let agent = Arc::new(LiveAgent::default());
        install_runtime(
            &settings,
            InboxRuntime {
                agent_turn_backend: agent.clone(),
                publish: Arc::new(|_| {}),
            },
        );
        Self {
            temp,
            settings,
            project,
            agent,
        }
    }
    fn repository(&self) -> ConversationRepository {
        ConversationRepository::new(crate::shared_store(&self.settings.data_dir))
    }
    fn handle(&self, conversation_id: &str) -> String {
        self.repository()
            .find_conversation_handle(conversation_id)
            .unwrap()
            .unwrap()
    }
    /// A new chat in `project`; returns its handle.
    fn chat(&self, conversation_id: &str, project: &str) -> String {
        self.repository()
            .touch_conversation(conversation_id, project)
            .unwrap();
        self.handle(conversation_id)
    }
    /// `spark send --to <to> --conversation <from> --message <message>`.
    fn send(&self, from: &str, to: &str, message: &str) -> Result<Value, WorkspaceError> {
        send_message(
            &self.settings,
            &crate::shared_store(&self.settings.data_dir),
            to,
            ThreadMessageRequest {
                conversation_handle: from.into(),
                message: message.into(),
            },
        )
    }
    fn requests(&self) -> Vec<AgentTurnRequest> {
        self.agent.gated.requests.lock().unwrap().clone()
    }
    /// Waits for a turn of `conversation_id` whose prompt contains `text`.
    fn wait_prompt(&self, conversation_id: &str, text: &str) -> AgentTurnRequest {
        let mut found = None;
        wait_for(&format!("{text:?} in {conversation_id}"), || {
            found = self.requests().into_iter().find(|request| {
                request.conversation_id == conversation_id && request.prompt.contains(text)
            });
            found.is_some()
        });
        found.unwrap()
    }
    /// The finished result a completed executor writes over the pending one.
    fn finish(&self, run_id: &str, status: &str, error: Option<&str>) {
        let store = RunStore::for_settings(&self.settings);
        let paths = store.find_run_root(run_id).unwrap().unwrap();
        let mut record = store.read_run_record(&paths).unwrap().unwrap();
        record.status = status.into();
        record.last_error = error.unwrap_or_default().into();
        store.write_run_record(&paths, &record).unwrap();
        store
            .write_result(&paths, &RunResult::unavailable(run_id, status))
            .unwrap();
    }
}

fn seeded(run_id: &str, outcome: &str) -> String {
    format!("Run {run_id} (work/seeded, \"Seeded work\") {outcome}.")
}

#[test]
fn chats_hear_every_outcome_of_the_runs_they_own_and_nothing_else() {
    let h = Harness::new();
    h.agent.gated.release(100);
    let chat = "chat-runs";
    h.chat(chat, &h.project);
    h.chat("chat-other", &h.project);
    for (run_id, status, error) in [
        ("run-done", "completed", None),
        ("run-broke", "failed", Some("disk full")),
        ("run-stopped", "canceled", None),
    ] {
        crate::owned_run(&h.settings, &h.project, chat, run_id, "running");
        h.finish(run_id, status, error);
        assert!(crate::deliver_run_events(&h.settings, run_id));
    }
    let store = RunStore::for_settings(&h.settings);
    crate::owned_run(&h.settings, &h.project, chat, "run-stuck", "running");
    let paths = store.find_run_root("run-stuck").unwrap().unwrap();
    let mut record = store.read_run_record(&paths).unwrap().unwrap();
    record.status = "waiting".into();
    record.outcome_reason_code = Some("recovery_decision_required".into());
    store.write_run_record(&paths, &record).unwrap();
    assert!(crate::deliver_run_events(&h.settings, "run-stuck"));
    // A child's question reaches its root's owner, even when the child emits
    // no flow name of its own.
    crate::owned_run(&h.settings, &h.project, chat, "run-root", "running");
    let mut child = RunRecord::new("run-child", &h.project);
    child.parent_run_id = Some("run-root".into());
    child.root_run_id = Some("run-root".into());
    child.status = "waiting".into();
    let child = store
        .create_run(CreateRunRequest {
            record: child,
            ..Default::default()
        })
        .unwrap();
    store
        .append_event(
            &child,
            attractor_runtime::human_gate_pending_event(
                "run-child",
                "gate-0",
                "gate",
                "",
                "Proceed?",
                None,
                vec![],
            ),
        )
        .unwrap();
    assert!(crate::deliver_run_events(&h.settings, "run-child"));
    // A run nobody owns reports nowhere.
    let unowned = store
        .create_run(CreateRunRequest {
            record: RunRecord::new("run-unowned", &h.project),
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(
            &unowned,
            &RunResult::unavailable("run-unowned", "completed"),
        )
        .unwrap();
    assert!(!crate::deliver_run_events(&h.settings, "run-unowned"));

    for text in [
        seeded("run-done", "ended completed"),
        seeded("run-broke", "ended failed: disk full"),
        seeded("run-stopped", "ended canceled"),
        seeded("run-stuck", "is waiting on a recovery decision"),
    ] {
        h.wait_prompt(chat, &text);
    }
    // The question names its root run's flow and launch summary, and keeps
    // the ids its answer is routed by.
    let asked = "[event question:run-child:gate-0 from run-child]\n\
                 Run question in run run-root (work/seeded, \"Seeded work\"): ";
    let prompt = h.wait_prompt(chat, asked).prompt;
    let (_, question) = prompt.split_once(asked).unwrap();
    let question: Value = serde_json::from_str(question.lines().next().unwrap()).unwrap();
    assert_eq!(
        [
            &question["run_id"],
            &question["question_id"],
            &question["root_run_id"],
            &question["prompt"]
        ],
        [
            &json!("run-child"),
            &json!("gate-0"),
            &json!("run-root"),
            &json!("Proceed?")
        ]
    );
    let requests = h.requests();
    assert!(requests
        .iter()
        .all(|request| request.conversation_id == chat && request.prompt.starts_with("[Inbox: ")));
    assert!(h
        .repository()
        .inbox_events("chat-other")
        .unwrap()
        .is_empty());
    assert!(!requests
        .iter()
        .any(|request| request.prompt.contains("run-unowned")));
}

#[test]
fn inbox_batches_start_idle_turns_join_live_ones_and_drain_after_a_user_turn() {
    let h = Harness::new();
    let chat = "chat-busy";
    let handle = h.chat(chat, &h.project);
    let sender = h.chat("chat-sender", &h.project);
    let conversations = crate::conversations_with_backend(h.settings.clone(), h.agent.clone());
    let (prepared, _) = conversations
        .send_turn(
            chat,
            ConversationTurnRequest {
                project_path: h.project.clone(),
                message: "Hello".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let prepared = prepared.unwrap();
    let user_turn = std::thread::spawn(move || {
        conversations.complete_started_turn_with_progress_payloads(prepared, |_| {})
    });
    wait_for("the user turn", || h.requests().len() == 1);
    // A chat's frame explains its reports and how it answers run questions.
    let frame = h.requests()[0].metadata[AGENT_INSTRUCTIONS_METADATA_KEY]
        .as_str()
        .unwrap()
        .to_string();
    assert!(frame.contains("Every run you launch reports back to this conversation."));
    assert!(frame.contains("it is not a new request from the user"));
    assert!(frame.contains(
        "When a run or descendant asks a question, you answer it. First decide whether you have the information to: this conversation,"
    ));
    assert!(frame.contains(&format!(
        "`spark send --to <handle> --conversation {handle}"
    )));

    // A running turn that takes live messages gets the batch live.
    h.agent.live.store(true, Ordering::SeqCst);
    h.send(&sender, &handle, "Live one").unwrap();
    let taken = h.agent.taken.lock().unwrap().clone();
    assert_eq!(taken.len(), 1);
    assert!(taken[0].starts_with("[Inbox: 1 event "), "{}", taken[0]);
    assert!(taken[0].contains(&format!(
        "Message from conversation {sender}, project {}:\nLive one",
        h.project
    )));
    // One it refuses queues the next turn; later events wait for the turn after.
    h.agent.live.store(false, Ordering::SeqCst);
    h.send(&sender, &handle, "Queued one").unwrap();
    h.send(&sender, &handle, "Waiting one").unwrap();
    let pending = h.repository().pending_inbox_events(chat).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].payload["message"], "Waiting one");
    assert_eq!(h.requests().len(), 1);

    // The user turn's completion drains the rest.
    h.agent.gated.release(100);
    user_turn.join().unwrap().unwrap();
    let queued = h.wait_prompt(chat, "Queued one");
    let waiting = h.wait_prompt(chat, "Waiting one");
    assert!(!queued.prompt.contains("Waiting one"));
    assert!(!waiting.prompt.contains("Queued one"));
    wait_for("an empty inbox", || {
        h.repository()
            .pending_inbox_events(chat)
            .unwrap()
            .is_empty()
    });
}

#[test]
fn chats_missions_and_roles_message_each_other_and_themselves_across_projects() {
    let h = Harness::new();
    h.agent.gated.release(1000);
    let chat = h.chat("chat-a", &h.project);
    let missions = crate::missions(h.settings.clone());
    let mission = missions
        .create(
            &h.project,
            serde_json::from_value(json!({"fields": {"title": "Ship"}})).unwrap(),
        )
        .unwrap();
    missions.start(&h.project, &mission.id).unwrap();
    let role = crate::roles(h.settings.clone())
        .start(&h.project, "coordinator")
        .unwrap();
    let kinds = [
        ("chat-a".to_string(), chat.clone(), String::new()),
        (
            mission.id.clone(),
            h.handle(&mission.id),
            " (mission \"Ship\")".to_string(),
        ),
        (
            role.conversation_id.clone(),
            h.handle(&role.conversation_id),
            " (standing role coordinator)".to_string(),
        ),
    ];
    for (from_id, from, label) in &kinds {
        for (to_id, to, _) in &kinds {
            let text = format!("From {from_id} to {to_id}");
            let accepted = h.send(from, to, &text).unwrap();
            assert_eq!(accepted["conversation_id"], json!(to_id));
            h.wait_prompt(
                to_id,
                &format!(
                    "Message from conversation {from}{label}, project {}:\n{text}",
                    h.project
                ),
            );
        }
    }
    let other = h.temp.path().canonicalize().unwrap().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let other = other.to_str().unwrap().to_string();
    ProjectRegistry::new(&h.settings.data_dir)
        .ensure_project_paths(&other)
        .unwrap();
    let remote = h.chat("chat-remote", &other);
    h.send(&remote, &chat, "From afar").unwrap();
    h.wait_prompt(
        "chat-a",
        &format!("Message from conversation {remote}, project {other}:\nFrom afar"),
    );
    h.send(&chat, &remote, "  And back  ").unwrap();
    h.wait_prompt(
        "chat-remote",
        &format!(
            "Message from conversation {chat}, project {}:\nAnd back",
            h.project
        ),
    );
    assert_eq!(
        h.repository().inbox_events("chat-remote").unwrap()[0].payload["message"],
        "And back"
    );
}

#[test]
fn a_launch_cannot_name_another_owner() {
    let h = Harness::new();
    crate::install_named_flow(&h.settings, "ops/run", simple_flow());
    // A launch belongs to the assistant turn that asked for it.
    seed_conversation(&h.settings, &h.project, "chat-a");
    let chat = h.handle("chat-a");
    h.chat("chat-victim", &h.project);
    let conversations = crate::conversations(h.settings.clone());
    let launch = |handle: Option<String>| {
        let response = conversations
            .launch_workspace_run(RunLaunchRequest {
                flow_name: "ops/run".into(),
                summary: "Spoofed".into(),
                conversation_handle: handle,
                project_path: Some(h.project.clone()),
                launch_context: Some(
                    json!({"context.spark_owner": {"conversation_id": "chat-victim"}}),
                ),
                execution_profile_id: Some("native".into()),
                ..Default::default()
            })
            .unwrap();
        response["run_id"].as_str().unwrap().to_string()
    };
    crate::assert_owned(&h.settings, &launch(Some(chat)), "chat-a");
    assert_eq!(crate::run_owner(&h.settings, &launch(None)), Value::Null);
    assert!(h.repository().inbox_runs("chat-victim").unwrap().is_empty());
}

#[test]
fn sends_need_live_handles_and_text_and_each_one_is_a_new_message() {
    let h = Harness::new();
    h.agent.gated.release(100);
    let chat = h.chat("chat-a", &h.project);
    let sender = h.chat("chat-b", &h.project);
    for (from, to, message) in [(&sender, &chat, " \n "), (&"  ".to_string(), &chat, "Hi")] {
        assert!(matches!(
            h.send(from, to, message),
            Err(WorkspaceError::Validation(_))
        ));
    }
    for (from, to) in [
        (sender.as_str(), "no-such-handle"),
        ("no-such-handle", chat.as_str()),
    ] {
        assert!(matches!(
            h.send(from, to, "Hi"),
            Err(WorkspaceError::NotFound(_))
        ));
    }
    h.send(&sender, &chat, "Twice").unwrap();
    h.send(&sender, &chat, "Twice").unwrap();
    let events = h.repository().inbox_events("chat-a").unwrap();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].id, events[1].id);
    assert!(events
        .iter()
        .all(|event| event.kind == "thread.message" && event.source == "chat-b"));
    // A deleted conversation can neither receive nor send.
    h.repository()
        .delete_conversation("chat-b", &h.project)
        .unwrap();
    for (from, to) in [(&chat, &sender), (&sender, &chat)] {
        assert!(matches!(
            h.send(from, to, "Still there?"),
            Err(WorkspaceError::NotFound(_))
        ));
    }
}

#[test]
fn a_new_mission_holds_messages_and_triggers_in_its_conversation_until_it_starts() {
    let h = Harness::new();
    let chat = h.chat("chat-a", &h.project);
    let missions = crate::missions(h.settings.clone());
    let mission = missions
        .create(
            &h.project,
            serde_json::from_value(json!({"fields": {"title": "Later"}})).unwrap(),
        )
        .unwrap();
    // Creation makes the conversation and its handle, and calls no agent.
    let handle = h.handle(&mission.id);
    assert_eq!(
        h.repository()
            .conversation_project_path(&mission.id)
            .unwrap(),
        Some(h.project.clone())
    );
    assert_eq!(
        mission.conversation_handle.as_deref(),
        Some(handle.as_str())
    );
    assert!(mission.started_at.is_none());
    h.send(&chat, &handle, "Before start").unwrap();
    missions
        .deliver_trigger(&spark_triggers::TriggerActivationRequest {
            trigger_id: "trigger-early".into(),
            project_path: h.project.clone(),
            trigger_name: "Early".into(),
            source_type: "webhook".into(),
            action: serde_json::from_value(
                json!({"mode": "mission", "mission_id": mission.id, "static_context": {}}),
            )
            .unwrap(),
            source_payload: json!({"n": 1}),
            delivery_id: Some("delivery-1".into()),
        })
        .unwrap();
    crate::recover(&h.settings);
    assert!(h.requests().is_empty());
    assert_eq!(
        h.repository()
            .pending_inbox_events(&mission.id)
            .unwrap()
            .len(),
        2
    );
    h.agent.gated.release(1);
    missions.start(&h.project, &mission.id).unwrap();
    let begin = h.wait_prompt(&mission.id, "Before start");
    assert!(
        begin
            .prompt
            .starts_with("Begin work on this mission.\n\n[Inbox: 2 events "),
        "{}",
        begin.prompt
    );
    assert!(begin.prompt.contains("Trigger fired: \"Early\""));
    assert_eq!(h.handle(&mission.id), handle);
}

#[test]
fn a_mission_opened_from_a_chat_reports_its_close_to_the_chat() {
    let h = Harness::new();
    h.agent.gated.release(10);
    let chat = h.chat("chat-a", &h.project);
    let missions = crate::missions(h.settings.clone());
    let mission = missions
        .create_from_conversation(
            &chat,
            None,
            serde_json::from_value(json!({"fields": {"title": "Ship"}})).unwrap(),
        )
        .unwrap();
    missions
        .close(
            &h.project,
            &mission.id,
            serde_json::from_value(json!({"reason": "Shipped"})).unwrap(),
        )
        .unwrap();
    h.wait_prompt(
        "chat-a",
        &format!("Mission \"Ship\" ({}) closed as done: Shipped", mission.id),
    );
}

#[test]
fn a_mission_closed_or_canceled_during_a_turn_keeps_its_queued_batch_without_a_turn() {
    for cancel in [false, true] {
        let h = Harness::new();
        // The completion thread's last step processes the mission, which
        // publishes it: past that point no queued turn can still start.
        let upserts = Arc::new(Mutex::new(0));
        let counted = upserts.clone();
        install_runtime(
            &h.settings,
            InboxRuntime {
                agent_turn_backend: h.agent.clone(),
                publish: Arc::new(move |envelope| {
                    if envelope.event_type == "mission.upsert" {
                        *counted.lock().unwrap() += 1;
                    }
                }),
            },
        );
        let chat = h.chat("chat-a", &h.project);
        let missions = crate::missions(h.settings.clone());
        let mission = missions
            .create(
                &h.project,
                serde_json::from_value(json!({"fields": {"title": "Ship"}})).unwrap(),
            )
            .unwrap();
        missions.start(&h.project, &mission.id).unwrap();
        h.wait_prompt(&mission.id, "Begin work on this mission.");
        // The busy turn refuses the message live, so the accepted batch queues.
        h.send(&chat, &h.handle(&mission.id), "Too late").unwrap();
        assert!(h
            .repository()
            .pending_inbox_events(&mission.id)
            .unwrap()
            .is_empty());
        if cancel {
            missions.cancel(&h.project, &mission.id).unwrap();
        } else {
            missions
                .close(
                    &h.project,
                    &mission.id,
                    serde_json::from_value(json!({"reason": "Done"})).unwrap(),
                )
                .unwrap();
        }
        let published = *upserts.lock().unwrap();
        h.agent.gated.release(1);
        wait_for("the completed turn's processing", || {
            *upserts.lock().unwrap() > published
        });
        assert_eq!(h.requests().len(), 1, "cancel: {cancel}");
        let turns = crate::conversations(h.settings.clone())
            .get_snapshot(&mission.id, Some(&h.project))
            .unwrap()["turns"]
            .clone();
        let queued = turns
            .as_array()
            .unwrap()
            .iter()
            .find(|turn| {
                turn["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("Too late"))
            })
            .unwrap();
        assert_eq!(queued["status"], "queued", "cancel: {cancel}");
        assert_eq!(
            missions.get(&h.project, &mission.id).unwrap().status,
            spark_workspace::missions::MissionStatus::Closed
        );
    }
}

#[test]
fn a_deleted_owner_discards_its_runs_reports() {
    let h = Harness::new();
    h.chat("chat-gone", &h.project);
    crate::owned_run(&h.settings, &h.project, "chat-gone", "run-late", "running");
    h.finish("run-late", "completed", None);
    h.repository()
        .delete_conversation("chat-gone", &h.project)
        .unwrap();
    assert!(!crate::deliver_run_events(&h.settings, "run-late"));
    crate::recover(&h.settings);
    assert!(h.requests().is_empty());
    assert!(h
        .repository()
        .conversation_project_path("chat-gone")
        .unwrap()
        .is_none());
}

#[test]
fn a_mission_whose_conversation_cannot_be_created_is_not_created() {
    let h = Harness::new();
    let published = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = published.clone();
    install_runtime(
        &h.settings,
        InboxRuntime {
            agent_turn_backend: h.agent.clone(),
            publish: Arc::new(move |envelope| {
                sink.lock()
                    .unwrap()
                    .push(serde_json::to_value(envelope).unwrap())
            }),
        },
    );
    // The store refuses every new mission conversation.
    crate::conversation_seed::write_rows(&crate::shared_store(&h.settings.data_dir), |tx, _| {
        tx.execute_batch(
            "CREATE TRIGGER refuse_mission_conversations BEFORE INSERT ON conversations \
             WHEN NEW.id LIKE 'mission-%' \
             BEGIN SELECT RAISE(ABORT, 'injected materialization failure'); END;",
        )
    });
    let missions = crate::missions(h.settings.clone());
    let error = missions
        .create(
            &h.project,
            serde_json::from_value(json!({"fields": {"title": "Unaddressable"}})).unwrap(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected materialization failure"),
        "{error}"
    );
    assert!(missions.list(&h.project).unwrap().is_empty());
    let records = ProjectRegistry::new(&h.settings.data_dir)
        .project_paths(&h.project)
        .unwrap()
        .root
        .join("missions");
    assert!(std::fs::read_dir(&records)
        .map(|entries| entries
            .filter_map(Result::ok)
            .all(|entry| entry.path().extension().is_none_or(|ext| ext != "json")))
        .unwrap_or(true));
    assert!(published.lock().unwrap().is_empty());
    assert!(h.requests().is_empty());
}

#[test]
fn recovery_registers_an_owned_run_a_crash_left_out_of_its_roster_and_reports_it_once() {
    let h = Harness::new();
    h.agent.gated.release(10);
    let chat = "chat-crashed";
    h.chat(chat, &h.project);
    // The launch recorded its owner, then the process died before the
    // roster registration; the run finished with its result still pending.
    let mut record = RunRecord::new("run-crashed", &h.project);
    record.flow_name = "work/crashed".into();
    record.title = Some("Crashed work".into());
    record.status = "completed".into();
    record.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            json!({"conversation_id": chat}),
        )]
        .into(),
    );
    let store = RunStore::for_settings(&h.settings);
    let paths = store
        .create_run(CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(&paths, &RunResult::pending("run-crashed", "completed"))
        .unwrap();
    assert!(h.repository().inbox_runs(chat).unwrap().is_empty());

    crate::recover(&h.settings);
    let report = "Run run-crashed (work/crashed, \"Crashed work\") ended completed.";
    h.wait_prompt(chat, report);
    let runs = h.repository().inbox_runs(chat).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].run_id.as_str(), runs[0].status.as_str()),
        ("run-crashed", "completed")
    );
    wait_for("the report turn to end", || {
        h.repository()
            .pending_inbox_events(chat)
            .unwrap()
            .is_empty()
            && h.agent.gated.requests.lock().unwrap().len() == 1
    });
    // A second start finds nothing new.
    crate::recover(&h.settings);
    let events = h.repository().inbox_events(chat).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, "run-crashed:completed");
    assert_eq!(
        h.requests()
            .iter()
            .filter(|request| request.prompt.contains(report))
            .count(),
        1
    );
    assert!(h
        .requests()
        .iter()
        .all(|request| request.conversation_id == chat));
}
