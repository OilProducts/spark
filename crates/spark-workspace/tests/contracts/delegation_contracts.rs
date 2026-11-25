//! Whole-task handoff: the records A keeps of what it delegates, and how B
//! admits, launches, reports and seals a delegation, driven through the
//! workspace API with no network between them.
use super::mission_contracts::{project, wait_for, GatedAgent};
use super::review_artifact_contracts::{
    plan_completed, settings, simple_flow, write_native_execution_profile,
};
use attractor_core::{RunRecord, RunResult};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnOutput;
use spark_common::settings::SparkSettings;
use spark_storage::environment::{DelegationIn, EnvironmentLink, EnvironmentRepository};
use spark_storage::{ConversationRepository, ProjectRegistry};
use spark_workspace::delegations::{
    self, request_digest, DelegationStartRequest, IncomingCancel, IncomingDelegation,
    DELEGATION_CONTEXT_KEY,
};
use spark_workspace::inbox::{install_runtime, InboxRuntime};
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService, WorkspaceError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The linked environment A delegates to, and the origin B records.
const LINKED: &str = "environment-b";
const ORIGIN: &str = "environment-a";

struct Home {
    _temp: tempfile::TempDir,
    settings: SparkSettings,
    project: String,
    agent: Arc<GatedAgent>,
}

impl Home {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(&temp.path().canonicalize().unwrap());
        let project = project(&settings);
        ProjectRegistry::new(&settings.data_dir)
            .ensure_project_paths(&project)
            .unwrap();
        write_native_execution_profile(&settings);
        crate::install_named_flow(&settings, "ops/run", simple_flow());
        Self {
            _temp: temp,
            settings,
            project,
            agent: Arc::new(GatedAgent::default()),
        }
    }
    fn service(&self) -> WorkspaceConversationService {
        crate::conversations_with_backend(self.settings.clone(), self.agent.clone())
    }
    fn environments(&self) -> EnvironmentRepository {
        EnvironmentRepository::new(&self.settings.data_dir)
    }
    fn repository(&self) -> ConversationRepository {
        ConversationRepository::new(crate::shared_store(&self.settings.data_dir))
    }
    fn link(&self) {
        let link = EnvironmentLink {
            environment_id: LINKED.into(),
            origin: "https://b.example".into(),
            label: "B".into(),
            created_at: String::new(),
        };
        self.environments().save_link(&link, "token").unwrap();
    }
    fn start(
        &self,
        request: DelegationStartRequest,
    ) -> Result<spark_storage::environment::DelegationOut, WorkspaceError> {
        delegations::start(
            &self.settings,
            &crate::shared_store(&self.settings.data_dir),
            request,
        )
    }
    fn flow_start(&self, request_key: &str, summary: &str) -> DelegationStartRequest {
        DelegationStartRequest {
            environment_id: LINKED.into(),
            project_path: "/b/project".into(),
            kind: "flow".into(),
            request_key: request_key.into(),
            flow_name: Some("ops/run".into()),
            summary: Some(summary.into()),
            ..Default::default()
        }
    }
    fn incoming(&self, id: &str, kind: &str, request: Value) -> IncomingDelegation {
        IncomingDelegation {
            delegation_id: id.into(),
            origin_environment_id: ORIGIN.into(),
            project_path: self.project.clone(),
            kind: kind.into(),
            request_sha256: request_digest(&self.project, kind, &request),
            request,
        }
    }
    fn admit(&self, request: IncomingDelegation) -> Result<Value, WorkspaceError> {
        delegations::admit(&self.service(), request)
    }
    fn state(&self, id: &str) -> Value {
        delegations::incoming_state(&self.service(), id).unwrap()
    }
    fn runs(&self) -> Vec<RunRecord> {
        RunStore::for_settings(&self.settings)
            .list_run_records()
            .unwrap()
    }
    fn run(&self, run_id: &str) -> RunRecord {
        let store = RunStore::for_settings(&self.settings);
        let paths = store.find_run_root(run_id).unwrap().unwrap();
        store.read_run_record(&paths).unwrap().unwrap()
    }
    /// A root run record with `status` and the given result.
    fn seed_run(&self, run_id: &str, status: &str, result: RunResult) {
        let mut record = RunRecord::new(run_id, &self.project);
        record.status = status.into();
        let store = RunStore::for_settings(&self.settings);
        let paths = store
            .create_run(CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap();
        store.write_result(&paths, &result).unwrap();
    }
    /// A row B admitted and launched at `target_id`, as a seal test's start.
    fn launched(&self, id: &str, target_kind: &str, target_id: &str) {
        self.environments()
            .admit_in(&DelegationIn {
                delegation_id: id.into(),
                origin_environment_id: ORIGIN.into(),
                project_path: self.project.clone(),
                kind: if target_kind == "run" {
                    "flow"
                } else {
                    "conversation"
                }
                .into(),
                request: json!({}),
                request_sha256: "seeded".into(),
                target_kind: target_kind.into(),
                target_id: target_id.into(),
                phase: "launched".into(),
                result: None,
                created_at: String::new(),
                sealed_at: None,
                canceled_at: None,
            })
            .unwrap();
    }
    /// A conversation whose last assistant turn has `status`, with `segments`.
    fn seed_chat(&self, id: &str, handle: &str, chat_mode: &str, status: &str, segments: Value) {
        crate::write_conversation_snapshot(
            &self.settings.data_dir,
            &json!({
                "schema_version": 5,
                "settings_schema_version": 1,
                "revision": 0,
                "conversation_id": id,
                "conversation_handle": handle,
                "project_path": self.project,
                "chat_mode": chat_mode,
                "provider": "codex",
                "model": null,
                "llm_profile": null,
                "reasoning_effort": null,
                "title": "Delegated",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:01Z",
                "turns": [
                    {"id": "turn-user", "role": "user", "content": "Do the task.",
                     "timestamp": "2026-01-01T00:00:00Z", "status": "complete", "kind": "message"},
                    {"id": "turn-assistant", "role": "assistant", "content": "Task done.",
                     "timestamp": "2026-01-01T00:00:01Z", "status": status, "kind": "message"}
                ],
                "segments": segments,
                "event_log": [],
                "flow_run_requests": [],
                "flow_launches": [],
                "run_recoveries": [],
                "proposed_plans": []
            }),
        );
    }
}

fn id(n: u8) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn flow_task(summary: &str) -> Value {
    json!({"flow_name": "ops/run", "summary": summary})
}

#[test]
fn a_start_is_keyed_by_caller_and_request_key() {
    let home = Home::new();
    let missing = home.start(home.flow_start(" ", "Build it"));
    assert!(
        matches!(missing, Err(WorkspaceError::Validation(_))),
        "{missing:?}"
    );
    let unlinked = home.start(home.flow_start("build", "Build it"));
    assert!(
        matches!(unlinked, Err(WorkspaceError::NotFound(_))),
        "{unlinked:?}"
    );

    home.link();
    let first = home.start(home.flow_start("build", "Build it")).unwrap();
    assert_eq!(
        (
            first.phase.as_str(),
            first.caller.as_str(),
            &first.owner_conversation_id
        ),
        ("created", "owner", &None)
    );
    let again = home.start(home.flow_start("build", "Build it")).unwrap();
    assert_eq!(again.delegation_id, first.delegation_id);
    let changed = home.start(home.flow_start("build", "Build something else"));
    assert!(
        matches!(changed, Err(WorkspaceError::Conflict(_))),
        "{changed:?}"
    );

    // A run's key is its own; its outcome reports to the run's owner.
    home.repository()
        .touch_conversation("chat-owner", &home.project)
        .unwrap();
    crate::owned_run(
        &home.settings,
        &home.project,
        "chat-owner",
        "run-live",
        "running",
    );
    let from_run = home
        .start(DelegationStartRequest {
            run_id: Some("run-live".into()),
            ..home.flow_start("build", "Build it")
        })
        .unwrap();
    assert_ne!(from_run.delegation_id, first.delegation_id);
    assert_eq!(from_run.caller, "run:run-live");
    assert_eq!(
        from_run.owner_conversation_id.as_deref(),
        Some("chat-owner")
    );

    // A conversation reports to itself.
    home.seed_chat("chat-caller", "amber-caller", "chat", "complete", json!([]));
    let from_chat = home
        .start(DelegationStartRequest {
            conversation_handle: Some("amber-caller".into()),
            ..home.flow_start("build", "Build it")
        })
        .unwrap();
    assert_eq!(from_chat.caller, "conversation:chat-caller");
    assert_eq!(
        from_chat.owner_conversation_id.as_deref(),
        Some("chat-caller")
    );

    // Once the link is removed, its records pause; a retry still returns its
    // record or conflicts, and only a new task is refused.
    delegations::remove_link(&home.settings, LINKED).unwrap();
    let retried = home.start(home.flow_start("build", "Build it")).unwrap();
    assert_eq!(retried.delegation_id, first.delegation_id);
    assert!(retried.paused.is_some(), "{retried:?}");
    let changed = home.start(home.flow_start("build", "Build something else"));
    assert!(
        matches!(changed, Err(WorkspaceError::Conflict(_))),
        "{changed:?}"
    );
    let new = home.start(home.flow_start("deploy", "Deploy it"));
    assert!(matches!(new, Err(WorkspaceError::NotFound(_))), "{new:?}");
}

#[test]
fn a_refuses_callers_that_cannot_delegate() {
    let home = Home::new();
    home.link();
    home.repository()
        .touch_conversation("chat-owner", &home.project)
        .unwrap();
    crate::owned_run(
        &home.settings,
        &home.project,
        "chat-owner",
        "run-done",
        "completed",
    );
    crate::owned_run(
        &home.settings,
        &home.project,
        "chat-owner",
        "run-docker",
        "running",
    );
    let store = RunStore::for_settings(&home.settings);
    let paths = store.find_run_root("run-docker").unwrap().unwrap();
    let mut record = home.run("run-docker");
    record.execution_mode = "container".into();
    store.write_run_record(&paths, &record).unwrap();
    home.seed_chat("chat-plan", "amber-plan", "plan", "complete", json!([]));

    let from_run = |run_id: &str| {
        home.start(DelegationStartRequest {
            run_id: Some(run_id.into()),
            ..home.flow_start("build", "Build it")
        })
    };
    assert!(matches!(
        from_run("run-done"),
        Err(WorkspaceError::Conflict(_))
    ));
    assert!(matches!(
        from_run("run-docker"),
        Err(WorkspaceError::Forbidden(_))
    ));
    assert!(matches!(
        from_run("run-missing"),
        Err(WorkspaceError::NotFound(_))
    ));
    let planning = home.start(DelegationStartRequest {
        conversation_handle: Some("amber-plan".into()),
        ..home.flow_start("build", "Build it")
    });
    assert!(
        matches!(planning, Err(WorkspaceError::Forbidden(_))),
        "{planning:?}"
    );
    assert!(EnvironmentRepository::new(&home.settings.data_dir)
        .followed_out()
        .unwrap()
        .is_empty());
}

#[test]
fn b_admits_each_delegation_once() {
    let home = Home::new();
    let first = home
        .admit(home.incoming(&id(1), "flow", flow_task("Build it")))
        .unwrap();
    let target = first["target_id"].as_str().unwrap().to_string();
    assert_eq!(first["target_kind"], "run");
    let again = home
        .admit(home.incoming(&id(1), "flow", flow_task("Build it")))
        .unwrap();
    assert_eq!(again["target_id"], target.as_str());
    let changed = home.admit(home.incoming(&id(1), "flow", flow_task("Something else")));
    assert!(
        matches!(changed, Err(WorkspaceError::Conflict(_))),
        "{changed:?}"
    );
    let mut forged = home.incoming(&id(2), "flow", flow_task("Build it"));
    forged.request_sha256 = "0".repeat(64);
    assert!(matches!(
        home.admit(forged),
        Err(WorkspaceError::Validation(_))
    ));
    let not_uuid = home.admit(home.incoming("run-1", "flow", flow_task("Build it")));
    assert!(matches!(not_uuid, Err(WorkspaceError::Validation(_))));
    assert_eq!(home.runs().len(), 1);

    // The launched run sits at its fixed ID, marked with its provenance and
    // owned by nobody on B.
    let run = home.run(&target);
    let context = run.launch_context.unwrap_or_default();
    assert_eq!(
        context.get(DELEGATION_CONTEXT_KEY),
        Some(&json!({"environment_id": ORIGIN, "delegation_id": id(1)}))
    );
    assert!(!context.contains_key("context.spark_owner"));
    assert_eq!(
        (
            run.parent_run_id,
            run.root_run_id.filter(|root| root != &target)
        ),
        (None, None)
    );

    // The run ends on its own; the first view after that seals it.
    wait_for("the delegated run to seal", || {
        home.state(&id(1))["phase"] == "sealed"
    });
    let sealed = home.state(&id(1));
    assert_eq!(sealed["result"]["run_id"], target.as_str());
    assert_eq!(sealed["status"], sealed["result"]["status"]);
}

#[test]
fn b_seals_what_it_cannot_launch_and_tombstones_what_was_canceled_first() {
    let home = Home::new();
    let unknown_flow = home
        .admit(home.incoming(
            &id(1),
            "flow",
            json!({"flow_name": "ops/missing", "summary": "x"}),
        ))
        .unwrap();
    let mut unregistered = home.incoming(&id(2), "conversation", json!({"message": "Hi"}));
    unregistered.project_path = "/not/registered".into();
    unregistered.request_sha256 =
        request_digest("/not/registered", "conversation", &unregistered.request);
    let unregistered = home.admit(unregistered).unwrap();
    for state in [&unknown_flow, &unregistered] {
        assert_eq!(state["phase"], "sealed", "{state}");
        assert_eq!(state["result"]["status"], "validation_error", "{state}");
    }

    // A cancel names a delegation by UUID, as admission does; another ID
    // leaves nothing behind.
    for invalid in ["not-a-uuid", "../x", ""] {
        let refused = delegations::cancel_incoming(
            &home.service(),
            invalid,
            IncomingCancel {
                origin_environment_id: ORIGIN.into(),
            },
        );
        assert!(
            matches!(&refused, Err(WorkspaceError::Validation(e)) if e.contains("UUID")),
            "{invalid}: {refused:?}"
        );
        assert!(home.environments().in_(invalid).unwrap().is_none());
    }
    let tombstone = delegations::cancel_incoming(
        &home.service(),
        &id(3),
        IncomingCancel {
            origin_environment_id: ORIGIN.into(),
        },
    )
    .unwrap();
    assert_eq!(
        (&tombstone["phase"], &tombstone["status"]),
        (&json!("sealed"), &json!("canceled"))
    );
    let late = home
        .admit(home.incoming(&id(3), "flow", flow_task("Build it")))
        .unwrap();
    assert_eq!(late["result"], json!({"status": "canceled"}));
    assert!(home.runs().is_empty());
    assert!(home.agent.requests.lock().unwrap().is_empty());
}

#[test]
fn b_starts_a_conversation_task_as_a_new_chat_and_seals_its_answer() {
    let home = Home::new();
    let admitted = home
        .admit(home.incoming(
            &id(1),
            "conversation",
            json!({"message": "Summarize the repo."}),
        ))
        .unwrap();
    let target = admitted["target_id"].as_str().unwrap().to_string();
    assert_eq!(admitted["target_kind"], "conversation");
    let turn = home
        .repository()
        .read_turn(&target, &id(1))
        .unwrap()
        .unwrap();
    assert_eq!(
        (&turn["role"], &turn["content"]),
        (&json!("user"), &json!("Summarize the repo."))
    );
    wait_for("the turn to start", || {
        !home.agent.requests.lock().unwrap().is_empty()
    });
    assert_eq!(home.state(&id(1))["status"], "running");

    home.agent.release(1);
    wait_for("the chat to seal", || {
        home.state(&id(1))["phase"] == "sealed"
    });
    assert_eq!(
        home.state(&id(1))["result"],
        json!({"status": "complete", "text": "Noted.", "error": null})
    );
}

#[test]
fn b_seals_a_run_once_it_has_ended() {
    let home = Home::new();
    home.seed_run(
        "run-going",
        "running",
        RunResult::pending("run-going", "running"),
    );
    home.seed_run(
        "run-done",
        "completed",
        RunResult::pending("run-done", "completed"),
    );
    home.seed_run(
        "run-answered",
        "failed",
        RunResult {
            body_markdown: "It broke.".into(),
            ..RunResult::unavailable("run-answered", "failed")
        },
    );
    home.launched(&id(1), "run", "run-going");
    home.launched(&id(2), "run", "run-done");
    home.launched(&id(3), "run", "run-answered");
    home.launched(&id(4), "run", "run-deleted");

    let going = home.state(&id(1));
    assert_eq!(
        (&going["phase"], &going["status"]),
        (&json!("launched"), &json!("running"))
    );
    // Its executor is gone without a final result: the result is unavailable.
    let done = home.state(&id(2));
    assert_eq!(done["phase"], "sealed");
    assert_eq!(
        (&done["result"]["status"], &done["result"]["state"]),
        (&json!("completed"), &json!("unavailable"))
    );
    let answered = home.state(&id(3));
    assert_eq!(answered["result"]["body_markdown"], "It broke.");
    assert_eq!(home.state(&id(4))["result"]["status"], "failed");

    // The seal stands whatever the run does next.
    let store = RunStore::for_settings(&home.settings);
    let paths = store.find_run_root("run-done").unwrap().unwrap();
    let mut record = home.run("run-done");
    record.status = "running".into();
    store.write_run_record(&paths, &record).unwrap();
    assert_eq!(home.state(&id(2))["result"], done["result"]);
}

#[test]
fn b_seals_a_conversation_once_it_is_idle_with_an_answer() {
    let home = Home::new();
    let question = json!([{
        "id": "segment-question", "turn_id": "turn-assistant", "order": 1,
        "kind": "request_user_input", "role": "assistant", "status": "pending",
        "timestamp": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "content": "",
        "request_user_input": {"request_id": "question-1", "status": "pending", "questions": [
            {"id": "decision", "header": "Approve", "question": "Approve?", "isOther": false,
             "isSecret": false, "options": [{"label": "Approve", "description": "Go"}]}]}
    }]);
    home.seed_chat("chat-idle", "amber-idle", "chat", "complete", json!([]));
    home.seed_chat("chat-failed", "amber-failed", "chat", "failed", json!([]));
    home.seed_chat("chat-asking", "amber-asking", "chat", "complete", question);
    home.seed_chat(
        "chat-working",
        "amber-working",
        "chat",
        "streaming",
        json!([]),
    );
    home.seed_chat(
        "chat-waiting",
        "amber-waiting",
        "chat",
        "complete",
        json!([]),
    );
    home.repository()
        .register_inbox_run("chat-waiting", "run-roster", "ops/run", "Roster run")
        .unwrap();
    for (n, chat) in [
        "chat-idle",
        "chat-failed",
        "chat-asking",
        "chat-working",
        "chat-waiting",
    ]
    .into_iter()
    .enumerate()
    {
        home.launched(&id(n as u8 + 1), "conversation", chat);
    }

    assert_eq!(
        home.state(&id(1))["result"],
        json!({"status": "complete", "text": "Task done.", "error": null})
    );
    assert_eq!(home.state(&id(2))["result"]["status"], "failed");
    let asking = home.state(&id(3));
    assert_eq!(
        (&asking["phase"], &asking["status"]),
        (&json!("launched"), &json!("waiting"))
    );
    assert_eq!(asking["questions"][0]["question_id"], "question-1");
    assert_eq!(home.state(&id(4))["status"], "running");
    let waiting = home.state(&id(5));
    assert_eq!(
        (&waiting["phase"], &waiting["status"]),
        (&json!("launched"), &json!("waiting"))
    );
}

#[test]
fn b_seals_a_canceled_conversation_that_waits_for_answers() {
    let home = Home::new();
    // An interrupted execution leaves its turn failed and its question open.
    let question = json!([{
        "id": "segment-question", "turn_id": "turn-assistant", "order": 1,
        "kind": "request_user_input", "role": "assistant", "status": "pending",
        "timestamp": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "content": "",
        "request_user_input": {"request_id": "question-1", "status": "pending", "questions": [
            {"id": "decision", "header": "Approve", "question": "Approve?", "isOther": false,
             "isSecret": false, "options": [{"label": "Approve", "description": "Go"}]}]}
    }]);
    home.seed_chat("chat-asking", "amber-asking", "chat", "failed", question);
    home.seed_chat("chat-done", "amber-done", "chat", "complete", json!([]));
    home.launched(&id(1), "conversation", "chat-asking");
    home.launched(&id(2), "conversation", "chat-done");
    assert_eq!(home.state(&id(1))["status"], "waiting");

    // B recorded the cancel and stopped before it sealed; its next read
    // expires the question and seals, and a repeated cancel returns the seal.
    assert!(home.environments().cancel_in(&id(1)).unwrap());
    let sealed = home.state(&id(1));
    assert_eq!(sealed["phase"], "sealed");
    assert_eq!(sealed["result"]["status"], "canceled");
    let snapshot = home
        .repository()
        .read_snapshot("chat-asking", None)
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot["segments"][0]["request_user_input"]["status"],
        "expired"
    );
    let cancel = || {
        delegations::cancel_incoming(&home.service(), &id(1), IncomingCancel::default()).unwrap()
    };
    assert_eq!(cancel(), sealed);

    // A conversation that ended before the cancel keeps its outcome.
    let done =
        delegations::cancel_incoming(&home.service(), &id(2), IncomingCancel::default()).unwrap();
    assert_eq!(done["result"]["status"], "complete");
    assert!(home
        .environments()
        .in_(&id(2))
        .unwrap()
        .unwrap()
        .canceled_at
        .is_none());
}

#[test]
fn b_seals_a_canceled_conversation_that_waits_for_plan_review() {
    let home = Home::new();
    let service = home.service();
    let admitted = home
        .admit(home.incoming(
            &id(1),
            "conversation",
            json!({"message": "Summarize the repo."}),
        ))
        .unwrap();
    let target = admitted["target_id"].as_str().unwrap().to_string();
    // The delegated turn ends while nothing reads the delegation.
    home.agent.release(1);
    wait_for("the delegated turn to end", || {
        let snapshot = home.repository().read_snapshot(&target, None).unwrap();
        snapshot.unwrap()["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|turn| turn["role"] == "assistant" && turn["status"] == "complete")
    });
    // B's owner then plans in the same chat, and the plan awaits review.
    let (prepared, _) = service
        .start_turn(
            &target,
            ConversationTurnRequest {
                project_path: home.project.clone(),
                message: "Plan the next step.".into(),
                chat_mode: Some("plan".into()),
                ..ConversationTurnRequest::default()
            },
        )
        .unwrap();
    service
        .ingest_agent_turn_output(
            &target,
            &home.project,
            &prepared.assistant_turn_id,
            "plan",
            AgentTurnOutput {
                events: vec![plan_completed("# Next Step\n\n1. Do it.")],
                ..AgentTurnOutput::default()
            },
        )
        .unwrap();
    assert_eq!(home.state(&id(1))["status"], "waiting");

    // The cancel rejects the plan and seals what the chat reached.
    let cancel =
        || delegations::cancel_incoming(&service, &id(1), IncomingCancel::default()).unwrap();
    let sealed = cancel();
    assert_eq!(sealed["phase"], "sealed");
    assert_eq!(sealed["result"]["status"], "complete");
    let snapshot = home
        .repository()
        .read_snapshot(&target, None)
        .unwrap()
        .unwrap();
    let plan = &snapshot["proposed_plans"][0];
    assert_eq!(
        (&plan["status"], &plan["review_note"]),
        (&json!("rejected"), &json!("The delegation was canceled."))
    );
    assert_eq!(cancel(), sealed);
}

#[test]
fn b_seals_a_conversation_only_after_the_turn_its_finished_run_starts() {
    let home = Home::new();
    install_runtime(
        &home.settings,
        InboxRuntime {
            agent_turn_backend: home.agent.clone(),
            runtime_handler_runner_factory: attractor_api::default_runtime_handler_runner_factory(),
            publish: Arc::new(|_| {}),
        },
    );
    let turns = || home.agent.requests.lock().unwrap().len();
    // Each round races B's reads against one run's completion.
    for n in 1..=30u8 {
        let target = home
            .admit(home.incoming(&id(n), "conversation", json!({"message": "Run the work."})))
            .unwrap()["target_id"]
            .as_str()
            .unwrap()
            .to_string();
        // The delegated turn launches a roster run and answers.
        let run_id = format!("run-{n}");
        crate::owned_run(&home.settings, &home.project, &target, &run_id, "running");
        home.agent.release(1);
        wait_for("the delegated turn to end", || {
            let snapshot = home.repository().read_snapshot(&target, None).unwrap();
            snapshot.unwrap()["turns"]
                .as_array()
                .unwrap()
                .iter()
                .any(|turn| turn["role"] == "assistant" && turn["status"] == "complete")
        });
        assert_eq!(home.state(&id(n))["status"], "waiting");

        // The run ends, and its report starts a turn the agent holds. B is
        // read throughout, and seals nothing while that turn runs.
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                while !stop.load(Ordering::SeqCst) {
                    let state = home.state(&id(n));
                    if state["phase"] == "sealed" {
                        return Some(state);
                    }
                }
                None
            });
            let store = RunStore::for_settings(&home.settings);
            let paths = store.find_run_root(&run_id).unwrap().unwrap();
            let mut record = store.read_run_record(&paths).unwrap().unwrap();
            record.status = "completed".into();
            store.write_run_record(&paths, &record).unwrap();
            store
                .write_result(&paths, &RunResult::unavailable(&run_id, "completed"))
                .unwrap();
            assert!(crate::deliver_run_events(&home.settings, &run_id));
            wait_for("the report's turn", || turns() == 2 * n as usize);
            stop.store(true, Ordering::SeqCst);
            assert_eq!(reader.join().unwrap(), None, "round {n}");
        });
        home.agent.release(1);
        wait_for("the conversation to seal", || {
            home.state(&id(n))["phase"] == "sealed"
        });
        assert_eq!(home.state(&id(n))["result"]["status"], "complete");
    }
}

#[test]
fn b_launches_accepted_rows_at_startup_once() {
    let home = Home::new();
    let accepted = |n: u8, target_id: &str| DelegationIn {
        delegation_id: id(n),
        origin_environment_id: ORIGIN.into(),
        project_path: home.project.clone(),
        kind: "flow".into(),
        request: flow_task("Build it"),
        request_sha256: request_digest(&home.project, "flow", &flow_task("Build it")),
        target_kind: "run".into(),
        target_id: target_id.into(),
        phase: "accepted".into(),
        result: None,
        created_at: String::new(),
        sealed_at: None,
        canceled_at: None,
    };
    // One row's launch never happened; the other's did, but not its mark.
    home.environments()
        .admit_in(&accepted(1, "run-fresh"))
        .unwrap();
    home.seed_run(
        "run-existing",
        "running",
        RunResult::pending("run-existing", "running"),
    );
    home.environments()
        .admit_in(&accepted(2, "run-existing"))
        .unwrap();

    delegations::launch_accepted(&home.service()).unwrap();
    delegations::launch_accepted(&home.service()).unwrap();
    let mut runs: Vec<String> = home.runs().into_iter().map(|run| run.run_id).collect();
    runs.sort();
    assert_eq!(runs, ["run-existing", "run-fresh"]);
    assert!(home.run("run-existing").launch_context.is_none());
    for n in [1, 2] {
        assert_ne!(
            home.environments().in_(&id(n)).unwrap().unwrap().phase,
            "accepted"
        );
    }

    // A cancel that reaches an unlaunched row seals it without a launch.
    home.environments()
        .admit_in(&accepted(3, "run-never"))
        .unwrap();
    let canceled =
        delegations::cancel_incoming(&home.service(), &id(3), IncomingCancel::default()).unwrap();
    assert_eq!(canceled["result"], json!({"status": "canceled"}));
    delegations::launch_accepted(&home.service()).unwrap();
    assert_eq!(home.runs().len(), 2);
}

#[test]
fn request_digest_ignores_key_order() {
    let forward: Value = serde_json::from_str(r#"{"a": 1, "b": {"x": 1, "y": 2}}"#).unwrap();
    let reversed: Value = serde_json::from_str(r#"{"b": {"y": 2, "x": 1}, "a": 1}"#).unwrap();
    assert_eq!(
        request_digest("/p", "flow", &forward),
        request_digest("/p", "flow", &reversed)
    );
}

/// The live turn a delegated chat's form answer reaches: it records the answer.
#[derive(Default)]
struct FormTaker(std::sync::Mutex<Vec<spark_agent_adapter::AgentRequestUserInputAnswerRequest>>);
impl spark_agent_adapter::AgentTurnBackend for FormTaker {
    fn run_turn(
        &self,
        _request: spark_agent_adapter::AgentTurnRequest,
    ) -> Result<AgentTurnOutput, spark_agent_adapter::AgentError> {
        unreachable!("only answers reach this agent")
    }
    fn answer_request_user_input(
        &self,
        request: spark_agent_adapter::AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, spark_agent_adapter::AgentError> {
        self.0.lock().unwrap().push(request);
        let delivered = spark_common::events::TurnStreamEvent::new(
            spark_common::events::TurnStreamEventKind::Other(
                "request_user_input_answer_delivered".into(),
            ),
        )
        .unwrap();
        Ok(AgentTurnOutput {
            events: vec![delivered],
            ..AgentTurnOutput::default()
        })
    }
}

/// Answers question `question` of delegation `id` as its owner would.
fn answer(home: &Home, id: &str, question: &str, request: Value) -> Result<Value, WorkspaceError> {
    answer_with(&home.service(), id, question, request)
}

fn answer_with(
    service: &WorkspaceConversationService,
    id: &str,
    question: &str,
    request: Value,
) -> Result<Value, WorkspaceError> {
    delegations::answer_incoming(
        service,
        id,
        question,
        serde_json::from_value(request).unwrap(),
    )
}

fn refused(result: Result<Value, WorkspaceError>, reason: &str) {
    match result {
        Err(WorkspaceError::Validation(error)) => assert!(error.contains(reason), "{error}"),
        other => panic!("expected a refusal with {reason:?}, got {other:?}"),
    }
}

#[test]
fn b_shows_a_delegated_form_and_takes_only_its_one_typed_reply() {
    let home = Home::new();
    let form = json!({"request_id": "acp-form-1", "harness": "acp:x", "harness_label": "X",
        "message": "Pick.", "schema": {"type": "object", "properties": {
            "count": {"type": "integer"}, "agree": {"type": "boolean"}}, "required": ["count"]}});

    // A conversation's form.
    home.seed_chat("chat-form", "amber-form", "chat", "complete", json!([{
        "id": "segment-form", "turn_id": "turn-assistant", "order": 1,
        "kind": "request_user_input", "role": "assistant", "status": "pending",
        "timestamp": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "content": "Pick.",
        "request_user_input": {"request_id": "acp-form-1", "status": "pending", "questions": [], "form": form}
    }]));
    home.launched(&id(1), "conversation", "chat-form");
    assert_eq!(
        home.state(&id(1))["questions"],
        json!([{"question_id": "acp-form-1", "questions": [], "form": form}])
    );
    for (request, reason) in [
        (json!({"selected_value": "3"}), "is a form: reply with form"),
        (
            json!({"answers": {"count": "3"}}),
            "is a form: reply with form",
        ),
        (
            json!({"form": {"action": "accept", "content": {}}}),
            "count is required.",
        ),
        (
            json!({"form": {"action": "accept", "content": {"count": "3"}}}),
            "whole number",
        ),
        (
            json!({"selected_value": "3", "form": {"action": "decline"}}),
            "exactly one",
        ),
    ] {
        refused(answer(&home, &id(1), "acp-form-1", request), reason);
    }
    assert_eq!(
        home.state(&id(1))["questions"][0]["question_id"],
        "acp-form-1"
    );
    let reply = json!({"action": "accept", "content": {"count": 0, "agree": false}});
    let agent = Arc::new(FormTaker::default());
    let service = crate::conversations_with_backend(home.settings.clone(), agent.clone());
    answer_with(&service, &id(1), "acp-form-1", json!({"form": reply})).unwrap();
    let reached = agent.0.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&reached[0].form).unwrap(),
        reply,
        "the agent gets the typed reply"
    );
    let snapshot = home
        .repository()
        .read_snapshot("chat-form", None)
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot["segments"][0]["request_user_input"]["reply"],
        reply
    );
    assert_eq!(home.state(&id(1))["questions"], json!([]));

    // A flow run's form.
    let store = RunStore::for_settings(&home.settings);
    let mut record = RunRecord::new("run-form", &home.project);
    record.status = "waiting".into();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    let active = attractor_runtime::clarification::ActiveClarification::new(&paths).unwrap();
    let mut gate = attractor_runtime::human_gate_pending_event(
        "run-form",
        "form-q",
        "work",
        "Flow",
        "Pick.",
        None,
        vec![],
    );
    gate.payload.extend([
        ("origin".to_string(), json!("agent_clarification")),
        ("clarification_id".to_string(), json!(active.id)),
        ("agent_request_id".to_string(), json!("acp-form-1")),
        ("question_type".to_string(), json!("FORM")),
        ("form".to_string(), form.clone()),
    ]);
    store.append_event(&paths, gate).unwrap();
    home.launched(&id(2), "run", "run-form");
    let asked = home.state(&id(2))["questions"][0].clone();
    assert_eq!(
        (&asked["question_id"], &asked["form"]),
        (&json!("form-q"), &form)
    );
    for (request, reason) in [
        (json!({"selected_value": "3"}), "This question is a form"),
        (
            json!({"form": {"action": "accept", "content": {"agree": true}}}),
            "count is required.",
        ),
    ] {
        refused(answer(&home, &id(2), "form-q", request), reason);
    }
    assert_eq!(home.state(&id(2))["questions"][0]["question_id"], "form-q");
    answer(
        &home,
        &id(2),
        "form-q",
        json!({"form": {"action": "decline"}}),
    )
    .unwrap();
    assert_eq!(home.state(&id(2))["questions"], json!([]));
    drop(active);
}

#[test]
fn a_cancel_on_a_paused_delegation_seals_locally_and_notifies_owner() {
    let home = Home::new();
    home.link();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let store = crate::shared_store(&home.settings.data_dir);
    let started = home
        .start(DelegationStartRequest {
            conversation_handle: Some("amber-owner".into()),
            ..home.flow_start("build", "Build it")
        })
        .unwrap();
    delegations::remove_link(&home.settings, LINKED).unwrap();
    let paused = delegations::get(&home.settings, &started.delegation_id).unwrap();
    assert!(paused.paused.is_some(), "{paused:?}");

    let canceled = delegations::cancel(&home.settings, &store, &started.delegation_id).unwrap();
    assert_eq!(canceled.phase, "sealed");
    assert!(canceled.paused.is_none());
    assert!(canceled.cancel_requested_at.is_some());
    assert_eq!(
        canceled.result.as_ref().unwrap()["status"],
        json!("canceled")
    );

    // Re-linking never reopens a sealed delegation, and owner got one inbox event.
    home.link();
    let after_relink = delegations::get(&home.settings, &started.delegation_id).unwrap();
    assert_eq!(after_relink.phase, "sealed");
    let events = home.repository().inbox_events("chat-owner").unwrap();
    let results: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "delegation.result" && event.source == started.delegation_id)
        .collect();
    assert_eq!(results.len(), 1, "{events:?}");
    assert_eq!(results[0].payload["result"]["status"], json!("canceled"));
}

#[test]
fn follower_polls_distinct_environments_in_parallel() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Barrier;

    let home = Home::new();
    let store = crate::shared_store(&home.settings.data_dir);
    let barrier = Arc::new(Barrier::new(2));

    let serve_one = |barrier: Arc<Barrier>| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            // Both environments must be contacted concurrently to pass the barrier.
            barrier.wait();
            let body = r#"{"phase":"sealed","target_kind":"run","target_id":"r-1","status":"completed","questions":[],"result":{"status":"completed"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://127.0.0.1:{port}"), handle)
    };

    let (origin_1, h1) = serve_one(barrier.clone());
    let (origin_2, h2) = serve_one(barrier);
    for (env_id, origin) in [("env-1", origin_1), ("env-2", origin_2)] {
        home.environments()
            .save_link(
                &EnvironmentLink {
                    environment_id: env_id.into(),
                    origin,
                    label: env_id.into(),
                    created_at: String::new(),
                },
                "token",
            )
            .unwrap();
    }
    let d1 = home
        .start(DelegationStartRequest {
            environment_id: "env-1".into(),
            ..home.flow_start("k1", "Task 1")
        })
        .unwrap();
    let d2 = home
        .start(DelegationStartRequest {
            environment_id: "env-2".into(),
            ..home.flow_start("k2", "Task 2")
        })
        .unwrap();

    let mut follower = delegations::Follower::default();
    follower.tick(&home.settings, &store);
    h1.join().unwrap();
    h2.join().unwrap();

    for id in [d1.delegation_id, d2.delegation_id] {
        let record = delegations::get(&home.settings, &id).unwrap();
        assert_eq!(record.phase, "sealed", "{record:?}");
    }
}

#[test]
fn requests_without_an_adapter_id_are_named_by_their_segment_end_to_end() {
    let home = Home::new();
    // Two requests with no adapter request_id that ask the same question.
    let asked = |segment_id: &str, order: u32| {
        json!({
            "id": segment_id, "turn_id": "turn-assistant", "order": order,
            "kind": "request_user_input", "role": "assistant", "status": "pending",
            "timestamp": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "content": "",
            "request_user_input": {"status": "pending", "questions": [
                {"id": "decision", "header": "Approve", "question": "Approve?", "isOther": false,
                 "isSecret": false, "options": [{"label": "Approve", "description": "Go"},
                                                {"label": "Reject", "description": "Stop"}]}]}
        })
    };
    home.seed_chat(
        "chat-twice",
        "amber-twice",
        "chat",
        "complete",
        json!([asked("segment-1", 1), asked("segment-2", 2)]),
    );
    home.launched(&id(1), "conversation", "chat-twice");
    let open = |home: &Home| {
        home.state(&id(1))["questions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|question| question["question_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(open(&home), ["segment-1", "segment-2"]);

    // A, following B's real view, notifies its owner of each once however
    // often it polls.
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let b = home.service();
    let peer = Peer::serve(move |_, _| {
        (
            200,
            delegations::incoming_state(&b, &id(1)).expect("B's view"),
        )
    });
    home.link_to(LINKED, &peer);
    let out = home.start_on(LINKED, "ask", Some("amber-owner"));
    for _ in 0..3 {
        assert_eq!(
            home.tick(&mut delegations::Follower::default()),
            Vec::<String>::new()
        );
    }
    let notified: Vec<_> = home
        .repository()
        .inbox_events("chat-owner")
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "delegation.question")
        .map(|event| (event.id, event.payload["question"]["question_id"].clone()))
        .collect();
    assert_eq!(
        notified,
        [
            (
                format!("delegation:{out}:question:segment-1"),
                json!("segment-1")
            ),
            (
                format!("delegation:{out}:question:segment-2"),
                json!("segment-2")
            ),
        ]
    );

    // Answering the second, once the first is answered, answers only it; the
    // agent still gets each under the question's own ID.
    let agent = Arc::new(FormTaker::default());
    let service = crate::conversations_with_backend(home.settings.clone(), agent.clone());
    answer_with(
        &service,
        &id(1),
        "segment-1",
        json!({"selected_value": "Approve"}),
    )
    .unwrap();
    assert_eq!(open(&home), ["segment-2"]);
    answer_with(
        &service,
        &id(1),
        "segment-2",
        json!({"selected_value": "Reject"}),
    )
    .unwrap();
    assert_eq!(open(&home), Vec::<String>::new());
    let snapshot = home
        .repository()
        .read_snapshot("chat-twice", None)
        .unwrap()
        .unwrap();
    let answers: Vec<_> = snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|segment| {
            (
                segment["id"].clone(),
                segment["request_user_input"]["answers"].clone(),
            )
        })
        .collect();
    assert_eq!(
        answers,
        [
            (json!("segment-1"), json!({"decision": "Approve"})),
            (json!("segment-2"), json!({"decision": "Reject"})),
        ]
    );
    let reached = agent.0.lock().unwrap();
    let routed: Vec<_> = reached
        .iter()
        .map(|request| request.request_id.as_str())
        .collect();
    assert_eq!(routed, ["decision", "decision"]);
}

#[test]
fn a_lists_outgoing_delegations_with_filters() {
    let home = Home::new();
    home.link();
    home.environments()
        .save_link(
            &EnvironmentLink {
                environment_id: "env-other".into(),
                origin: "https://other.example".into(),
                label: "Other".into(),
                created_at: String::new(),
            },
            "token-2",
        )
        .unwrap();
    // Each filter keeps a different set: open and paused differ by `open`,
    // the environment and phase filters by `paused` and `sealed`.
    let paused = home.start(home.flow_start("k1", "Task 1")).unwrap();
    let sealed = home
        .start(DelegationStartRequest {
            environment_id: "env-other".into(),
            ..home.flow_start("k2", "Task 2")
        })
        .unwrap();
    home.start(home.flow_start("k3", "Task 3")).unwrap();
    home.environments()
        .pause_out(&paused.delegation_id, "Paused for test")
        .unwrap();
    home.environments()
        .seal_out(
            &sealed.delegation_id,
            &json!({"status": "completed"}),
            false,
        )
        .unwrap();

    let list = |environment_id: Option<&str>, phase: Option<&str>, open, paused| {
        let mut keys: Vec<_> = delegations::list(
            &home.settings,
            delegations::DelegationListFilter {
                environment_id: environment_id.map(str::to_string),
                phase: phase.map(str::to_string),
                open,
                paused,
            },
        )
        .unwrap()
        .into_iter()
        .map(|record| record.request_key)
        .collect();
        keys.sort();
        keys
    };
    assert_eq!(list(None, None, false, false), ["k1", "k2", "k3"]);
    assert_eq!(list(None, None, true, false), ["k1", "k3"]);
    assert_eq!(list(None, None, false, true), ["k1"]);
    assert_eq!(list(Some(LINKED), None, false, false), ["k1", "k3"]);
    assert_eq!(list(None, Some("sealed"), false, false), ["k2"]);
    assert!(list(Some("env-other"), Some("created"), false, false).is_empty());
    // A blank value does not filter, and surrounding space is ignored.
    assert_eq!(list(Some(" "), Some(""), false, false), ["k1", "k2", "k3"]);
    assert_eq!(list(Some(" environment-b "), None, false, true), ["k1"]);
    assert_eq!(list(None, Some(" created "), true, false), ["k1", "k3"]);

    let bad_phase = delegations::list(
        &home.settings,
        delegations::DelegationListFilter {
            phase: Some("bogus".into()),
            ..Default::default()
        },
    );
    assert!(matches!(bad_phase, Err(WorkspaceError::Validation(_))));
}

#[test]
fn a_proxies_project_and_flow_discovery_to_a_linked_environment() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let home = Home::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, content_type, body) in [
            (200, "application/json", r#"[{"project_path":"/b/repo"}]"#),
            (200, "application/json", r#"[{"name":"ops/run"}]"#),
            (
                200,
                "application/json",
                r#"{"name":"ops/run","files":[{"name":"flow.md","content":"---\nid: run\n---\n"}]}"#,
            ),
            (
                404,
                "application/json",
                r#"{"detail":"Unknown flow: missing"}"#,
            ),
            (401, "application/json", r#"{"detail":"Unknown token."}"#),
            (403, "application/json", r#"{"detail":"Revoked token."}"#),
            (503, "application/json", r#"{"detail":"Peer is starting."}"#),
            (200, "text/plain", "all good"),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
            let response = format!(
                "HTTP/1.1 {status} Answer\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
        seen
    });

    home.environments()
        .save_link(
            &EnvironmentLink {
                environment_id: "env-b".into(),
                origin: format!("http://127.0.0.1:{port}"),
                label: "B".into(),
                created_at: String::new(),
            },
            "peer-secret",
        )
        .unwrap();

    // A flow name that would name another route is refused before any
    // request; the server sees only the three below.
    for bad in [
        ".",
        "..",
        "/../",
        "a/../b",
        "ops/./run",
        "ops//run",
        "a\\b",
        "a\u{7}b",
    ] {
        let refused = delegations::link_flow_files(&home.settings, "env-b", bad, None);
        assert!(
            matches!(&refused, Err(WorkspaceError::Validation(_))),
            "{bad:?}: {refused:?}"
        );
    }
    let projects = delegations::link_projects(&home.settings, "env-b").unwrap();
    assert_eq!(projects[0]["project_path"], "/b/repo");
    let flows = delegations::link_flows(&home.settings, "env-b", Some("/b/repo")).unwrap();
    assert_eq!(flows[0]["name"], "ops/run");
    let files = delegations::link_flow_files(
        &home.settings,
        "env-b",
        "software-development/implement-change",
        Some("/b/repo"),
    )
    .unwrap();
    assert_eq!(files["files"][0]["name"], "flow.md");

    // The peer's refusals keep their meaning; an outage or a success that is
    // not JSON is this environment's 503, naming the peer.
    let origin = format!("http://127.0.0.1:{port}");
    let flow = |name: &str| delegations::link_flow_files(&home.settings, "env-b", name, None);
    match flow("missing") {
        Err(WorkspaceError::NotFound(detail)) => assert_eq!(detail, "Unknown flow: missing"),
        other => panic!("{other:?}"),
    }
    for (name, detail) in [
        ("unknown-token", "Unknown token."),
        ("revoked", "Revoked token."),
    ] {
        match flow(name) {
            Err(WorkspaceError::Forbidden(message)) => assert!(
                message.contains(detail) && message.contains("link it again"),
                "{message}"
            ),
            other => panic!("{name}: {other:?}"),
        }
    }
    match delegations::link_flows(&home.settings, "env-b", None) {
        Err(WorkspaceError::ServiceUnavailable(message)) => assert!(
            message.starts_with(&origin) && message.contains("Peer is starting."),
            "{message}"
        ),
        other => panic!("{other:?}"),
    }
    match delegations::link_projects(&home.settings, "env-b") {
        Err(WorkspaceError::ServiceUnavailable(message)) => assert!(
            message.starts_with(&origin)
                && message
                    .contains("/workspace/api/projects answered 200 with a body that is not JSON")
                && message.contains("all good"),
            "{message}"
        ),
        other => panic!("{other:?}"),
    }

    let seen = server.join().unwrap();
    assert!(seen[0].starts_with("GET /workspace/api/projects HTTP/"));
    assert!(seen[1].starts_with("GET /workspace/api/flows?project_path=%2Fb%2Frepo HTTP/"));
    assert!(seen[2].starts_with(
        "GET /workspace/api/flows/software-development%2Fimplement-change/files?project_path=%2Fb%2Frepo HTTP/"
    ));
    for request in &seen {
        let lower = request.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: bearer peer-secret"),
            "{request}"
        );
    }

    assert!(matches!(
        delegations::link_projects(&home.settings, "missing"),
        Err(WorkspaceError::NotFound(_))
    ));
}

/// A linked environment the test plays. Each request is recorded as
/// `"<what> <delegation_id>"`, where what is `submit`, `poll` or `cancel`,
/// and answered with `respond(what, delegation_id)`'s status and body.
struct Peer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
}

/// A whole HTTP answer of `status` with the JSON body `reply`.
fn json_answer(status: u16, reply: &Value) -> Vec<u8> {
    let reply = reply.to_string();
    format!(
        "HTTP/1.1 {status} Answer\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    )
    .into_bytes()
}

impl Peer {
    fn serve(respond: impl Fn(&str, &str) -> (u16, Value) + Send + Sync + 'static) -> Self {
        Self::serve_raw(move |what, id| {
            let (status, reply) = respond(what, id);
            json_answer(status, &reply)
        })
    }
    /// As `serve`, answering with `respond`'s whole HTTP answer.
    fn serve_raw(respond: impl Fn(&str, &str) -> Vec<u8> + Send + Sync + 'static) -> Self {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (seen, respond) = (requests.clone(), Arc::new(respond));
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let (seen, respond) = (seen.clone(), respond.clone());
                // One thread per request, so concurrent followers overlap here.
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_string();
                    let path = parts.next().unwrap_or_default().to_string();
                    let mut length = 0;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header.trim().is_empty() {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse().unwrap();
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let rest = path
                        .trim_start_matches("/workspace/api/delegations/incoming")
                        .trim_start_matches('/');
                    let (id, what) = match rest.split_once('/') {
                        Some((id, what)) => (id, what),
                        None if method == "GET" => (rest, "poll"),
                        None => (body["delegation_id"].as_str().unwrap_or_default(), "submit"),
                    };
                    seen.lock().unwrap().push(format!("{what} {id}"));
                    let _ = stream.write_all(&respond(what, id));
                });
            }
        });
        Self { origin, requests }
    }
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
    fn count(&self, request: &str) -> usize {
        self.requests()
            .iter()
            .filter(|seen| *seen == request)
            .count()
    }
}

/// B's view of a delegation in `phase` with `status`; a sealed one carries
/// its result.
fn b_state(id: &str, phase: &str, status: &str) -> Value {
    let mut state = json!({
        "delegation_id": id, "phase": phase, "target_kind": "run",
        "target_id": format!("run-{id}"), "status": status, "questions": [],
    });
    if phase == "sealed" {
        state["result"] = json!({"status": status});
    }
    state
}

impl Home {
    fn link_to(&self, environment_id: &str, peer: &Peer) {
        let link = EnvironmentLink {
            environment_id: environment_id.into(),
            origin: peer.origin.clone(),
            label: environment_id.into(),
            created_at: String::new(),
        };
        self.environments().save_link(&link, "token").unwrap();
    }
    /// Starts a flow delegation to `environment_id` under `key`, reporting
    /// to the conversation with handle `owner` when given.
    fn start_on(&self, environment_id: &str, key: &str, owner: Option<&str>) -> String {
        self.start(DelegationStartRequest {
            environment_id: environment_id.into(),
            conversation_handle: owner.map(str::to_string),
            ..self.flow_start(key, key)
        })
        .unwrap()
        .delegation_id
    }
    fn out(&self, id: &str) -> spark_storage::environment::DelegationOut {
        delegations::get(&self.settings, id).unwrap()
    }
    /// The `delegation.result` events `owner`'s inbox holds for `id`.
    fn results(&self, owner: &str, id: &str) -> Vec<spark_storage::conversation::InboxEvent> {
        self.repository()
            .inbox_events(owner)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "delegation.result" && event.source == id)
            .collect()
    }
    fn tick(&self, follower: &mut delegations::Follower) -> Vec<String> {
        follower.tick(
            &self.settings,
            &crate::shared_store(&self.settings.data_dir),
        )
    }
}

#[test]
fn follower_backs_off_only_the_record_that_fails() {
    let home = Home::new();
    let failing = Arc::new(Mutex::new(String::new()));
    let down = failing.clone();
    let b = Peer::serve(move |_, id| {
        if *down.lock().unwrap() == id {
            (500, json!({"detail": "down"}))
        } else {
            (200, b_state(id, "sealed", "completed"))
        }
    });
    let c = Peer::serve(|_, id| (200, b_state(id, "sealed", "completed")));
    home.link_to("env-b", &b);
    home.link_to("env-c", &c);
    let pair = [
        home.start_on("env-b", "k1", None),
        home.start_on("env-b", "k2", None),
    ];
    let other = home.start_on("env-c", "k3", None);
    // The record the follower reaches first on B keeps failing.
    let oldest = home
        .environments()
        .followed_out()
        .unwrap()
        .into_iter()
        .find(|record| record.environment_id == "env-b")
        .unwrap()
        .delegation_id;
    let sibling = pair.iter().find(|id| **id != oldest).unwrap().clone();
    *failing.lock().unwrap() = oldest.clone();

    let mut follower = delegations::Follower::default();
    let diagnostics = home.tick(&mut follower);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].contains(&oldest) && diagnostics[0].ends_with("down"),
        "{diagnostics:?}"
    );
    for id in [&sibling, &other] {
        assert_eq!(home.out(id).phase, "sealed", "{id}");
    }
    let held = home.out(&oldest);
    assert_eq!((held.phase.as_str(), held.paused), ("created", None));
    assert_eq!(b.count(&format!("submit {oldest}")), 1);
    assert_eq!(b.count(&format!("submit {sibling}")), 1);

    // Before its retry is due, the failing record is left alone.
    home.tick(&mut follower);
    assert_eq!(b.requests().len(), 2, "{:?}", b.requests());
    assert_eq!(home.out(&oldest).phase, "created");
}

#[test]
fn a_cancel_seals_locally_only_what_b_never_saw_and_sends_the_rest_on_relink() {
    let home = Home::new();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let store = crate::shared_store(&home.settings.data_dir);
    // B launches what it is sent, except one submission whose answer is
    // lost; it acknowledges a cancel while the target stops, then seals it.
    let lost = Arc::new(Mutex::new(String::new()));
    let canceled = Arc::new(Mutex::new(Vec::<String>::new()));
    let (lose, cancels) = (lost.clone(), canceled.clone());
    let b = Peer::serve(move |what, id| match what {
        "submit" if *lose.lock().unwrap() == id => (500, json!({"detail": "lost"})),
        "cancel" => {
            cancels.lock().unwrap().push(id.to_string());
            (200, b_state(id, "launched", "running"))
        }
        "poll" if cancels.lock().unwrap().iter().any(|c| c == id) => {
            (200, b_state(id, "sealed", "canceled"))
        }
        _ => (200, b_state(id, "launched", "running")),
    });
    home.link_to(LINKED, &b);
    let owner = Some("amber-owner");
    let before = home.start_on(LINKED, "before", owner);
    let after = home.start_on(LINKED, "after", owner);
    let maybe = home.start_on(LINKED, "maybe", owner);
    *lost.lock().unwrap() = maybe.clone();
    home.tick(&mut delegations::Follower::default());
    assert_eq!(home.out(&before).phase, "accepted");
    assert_eq!(home.out(&after).phase, "accepted");
    let possibly = home.out(&maybe);
    assert_eq!(possibly.phase, "created");
    assert!(possibly.submitted_at.is_some());

    // Canceled before the pause: the cancel waits for B.
    let cancel = |id: &str| delegations::cancel(&home.settings, &store, id).unwrap();
    assert_eq!(cancel(&before).phase, "accepted");
    let never = home.start_on(LINKED, "never", owner);
    delegations::remove_link(&home.settings, LINKED).unwrap();
    for id in [&before, &after, &maybe, &never] {
        assert!(home.out(id).paused.is_some(), "{id}");
    }

    // Canceled while paused: only the record never submitted seals here.
    let sealed = cancel(&never);
    assert_eq!(sealed.phase, "sealed");
    assert_eq!(sealed.result.unwrap()["status"], "canceled");
    for id in [&before, &after, &maybe] {
        let pending = cancel(id);
        assert_ne!(pending.phase, "sealed", "{id}");
        assert!(pending.paused.is_some(), "{id}");
        assert!(pending.cancel_requested_at.is_some(), "{id}");
        assert!(pending.cancel_acknowledged_at.is_none(), "{id}");
        assert!(home.results("chat-owner", id).is_empty(), "{id}");
    }

    // Relinked, the follower sends the pending cancels, never a submission,
    // and seals each on B's answer.
    home.link_to(LINKED, &b);
    let seen = b.requests().len();
    let mut follower = delegations::Follower::default();
    home.tick(&mut follower);
    let mut sent = b.requests().split_off(seen);
    sent.sort();
    let mut expected: Vec<_> = [&before, &after, &maybe]
        .map(|id| format!("cancel {id}"))
        .into();
    expected.sort();
    assert_eq!(sent, expected);
    for id in [&before, &after, &maybe] {
        let acknowledged = home.out(id);
        assert!(acknowledged.cancel_acknowledged_at.is_some(), "{id}");
        assert_eq!(acknowledged.phase, "accepted", "{id}");
    }
    home.tick(&mut follower);
    for id in [&before, &after, &maybe, &never] {
        let record = home.out(id);
        assert_eq!(record.phase, "sealed", "{id}");
        assert_eq!(record.result.unwrap()["status"], "canceled", "{id}");
        let results = home.results("chat-owner", id);
        assert_eq!(results.len(), 1, "{id}: {results:?}");
        assert_eq!(results[0].payload["result"]["status"], "canceled");
    }
    let settled = b.requests().len();
    home.tick(&mut follower);
    assert_eq!(b.requests().len(), settled);
    assert_eq!(b.count(&format!("submit {never}")), 0);
}

#[test]
fn a_relink_racing_a_paused_cancel_never_loses_the_cancel() {
    let home = Home::new();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let store = crate::shared_store(&home.settings.data_dir);
    let b = Peer::serve(|what, id| match what {
        // A cancel for a task B never admitted leaves a canceled tombstone.
        "cancel" => (200, b_state(id, "sealed", "canceled")),
        _ => (200, b_state(id, "launched", "running")),
    });
    let mut ids = Vec::new();
    for round in 0..8 {
        home.link_to(LINKED, &b);
        let id = home.start_on(LINKED, &format!("race-{round}"), Some("amber-owner"));
        delegations::remove_link(&home.settings, LINKED).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                delegations::cancel(&home.settings, &store, &id).unwrap();
            });
            scope.spawn(|| {
                barrier.wait();
                home.link_to(LINKED, &b);
            });
        });
        // Sealed locally before the relink, or left for the follower to send.
        let record = home.out(&id);
        assert!(record.cancel_requested_at.is_some());
        if record.phase != "sealed" {
            assert!(record.paused.is_none() && record.cancel_acknowledged_at.is_none());
        }
        ids.push(id);
    }
    home.tick(&mut delegations::Follower::default());
    for id in &ids {
        let record = home.out(id);
        assert_eq!(record.phase, "sealed", "{id}");
        assert_eq!(record.result.unwrap()["status"], "canceled");
        assert_eq!(home.results("chat-owner", id).len(), 1, "{id}");
    }
    assert!(
        b.requests().iter().all(|seen| seen.starts_with("cancel ")),
        "{:?}",
        b.requests()
    );
}

#[test]
fn a_pause_that_commits_just_before_a_cancel_is_written_seals_it_locally() {
    let home = Home::new();
    home.link();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let store = crate::shared_store(&home.settings.data_dir);
    let id = home.start_on(LINKED, "build", Some("amber-owner"));
    // An unlink holds the write lock as the cancel arrives, so the cancel
    // is written just after the pause commits.
    let mut unlink =
        rusqlite::Connection::open(home.settings.data_dir.join("state.sqlite3")).unwrap();
    let pause = unlink
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    pause
        .execute(
            "DELETE FROM environment_links WHERE environment_id = ?1",
            [LINKED],
        )
        .unwrap();
    pause
        .execute(
            "UPDATE delegations_out SET paused = 'The link to this environment was removed.' \
             WHERE delegation_id = ?1",
            [&id],
        )
        .unwrap();
    let canceled = std::thread::scope(|scope| {
        let cancel = scope.spawn(|| delegations::cancel(&home.settings, &store, &id).unwrap());
        // Time for the cancel to read the record before the pause commits; it
        // must decide from the record as it stands once its cancel is written.
        std::thread::sleep(std::time::Duration::from_millis(200));
        pause.commit().unwrap();
        cancel.join().unwrap()
    });
    assert_eq!(canceled.phase, "sealed", "{canceled:?}");
    assert_eq!(canceled.result.unwrap()["status"], "canceled");
    assert_eq!(canceled.submitted_at, None);
    assert_eq!(home.results("chat-owner", &id).len(), 1);
}

#[test]
fn a_created_cancel_acknowledged_before_b_answer_was_stored_is_sent_until_it_seals() {
    let home = Home::new();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    // B tombstones a cancel for a task it never admitted.
    let b = Peer::serve(|what, id| match what {
        "cancel" => (200, b_state(id, "sealed", "canceled")),
        _ => (200, b_state(id, "launched", "running")),
    });
    home.link_to(LINKED, &b);
    let id = home.start_on(LINKED, "build", Some("amber-owner"));
    // A crash after the cancel was acknowledged, before B's answer was stored.
    assert!(home.environments().request_cancel_out(&id).unwrap());
    assert!(home.environments().acknowledge_cancel_out(&id).unwrap());
    let wedged = home.out(&id);
    assert_eq!(wedged.phase, "created");
    assert!(wedged.cancel_requested_at.is_some() && wedged.cancel_acknowledged_at.is_some());

    let mut follower = delegations::Follower::default();
    assert_eq!(home.tick(&mut follower), Vec::<String>::new());
    let record = home.out(&id);
    assert_eq!(record.phase, "sealed", "{record:?}");
    assert_eq!(record.result.unwrap()["status"], "canceled");
    assert_eq!(home.results("chat-owner", &id).len(), 1);
    assert_eq!(b.requests(), [format!("cancel {id}")]);
}

#[test]
fn a_committed_cancel_stands_when_delivering_it_to_the_owner_fails() {
    let home = Home::new();
    install_runtime(
        &home.settings,
        InboxRuntime {
            agent_turn_backend: home.agent.clone(),
            runtime_handler_runner_factory: attractor_api::default_runtime_handler_runner_factory(),
            publish: Arc::new(|_| {}),
        },
    );
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    home.link();
    let store = crate::shared_store(&home.settings.data_dir);
    let id = home.start_on(LINKED, "build", Some("amber-owner"));
    delegations::remove_link(&home.settings, LINKED).unwrap();
    // The owner's conversation can no longer take a turn, so delivering
    // anything to it fails.
    rusqlite::Connection::open(home.settings.data_dir.join("state.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE conversations SET metadata = \
             json_set(metadata, '$.settings_schema_version', 2) WHERE id = 'chat-owner'",
            [],
        )
        .unwrap();

    for _ in 0..2 {
        let canceled = delegations::cancel(&home.settings, &store, &id).unwrap();
        assert_eq!(canceled.phase, "sealed");
        assert_eq!(canceled.result.unwrap()["status"], "canceled");
        let results = home.results("chat-owner", &id);
        assert_eq!(results.len(), 1, "{results:?}");
        assert!(results[0].delivered_at.is_none(), "{results:?}");
    }
    assert!(home.agent.requests.lock().unwrap().is_empty());
}

#[test]
fn a_panicking_follower_worker_is_reported_and_backs_off_its_records() {
    let home = Home::new();
    let healthy = Peer::serve(|_, id| (200, b_state(id, "sealed", "completed")));
    let broken = Peer::serve(|_, id| (200, b_state(id, "sealed", "completed")));
    home.link_to("env-ok", &healthy);
    home.link_to("env-broken", &broken);
    let ok = home.start_on("env-ok", "ok", None);
    let lost = home.start_on("env-broken", "lost", None);
    let mut follower = delegations::Follower::default();
    follower.panic_environment = Some("env-broken".into());

    let diagnostics = home.tick(&mut follower);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].contains("env-broken") && diagnostics[0].contains(&lost),
        "{diagnostics:?}"
    );
    assert_eq!(home.out(&ok).phase, "sealed");
    assert_eq!(home.out(&lost).phase, "created");
    // Backed off: the next tick does not reach the broken worker again.
    assert_eq!(home.tick(&mut follower), Vec::<String>::new());
    assert!(broken.requests().is_empty());
}

#[test]
fn a_panic_following_one_record_backs_off_only_that_record() {
    let home = Home::new();
    // B keeps the first record running and seals the rest.
    let running = Arc::new(Mutex::new(String::new()));
    let first = running.clone();
    let b = Peer::serve(move |_, id| {
        if *first.lock().unwrap() == id {
            (200, b_state(id, "launched", "running"))
        } else {
            (200, b_state(id, "sealed", "completed"))
        }
    });
    home.link_to(LINKED, &b);
    for key in ["d1", "d2", "d3"] {
        home.start_on(LINKED, key, None);
    }
    // In the order the follower takes them; the middle one panics.
    let ids: Vec<_> = home
        .environments()
        .followed_out()
        .unwrap()
        .into_iter()
        .map(|record| record.delegation_id)
        .collect();
    let [d1, d2, d3] = <[String; 3]>::try_from(ids).unwrap();
    *running.lock().unwrap() = d1.clone();
    let mut follower = delegations::Follower::default();
    follower.panic_delegation = Some(d2.clone());

    let diagnostics = home.tick(&mut follower);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert!(
        diagnostic.contains(LINKED)
            && diagnostic.contains(&d2)
            && !diagnostic.contains(&d1)
            && !diagnostic.contains(&d3),
        "{diagnostic}"
    );
    assert_eq!(home.out(&d1).phase, "accepted");
    assert_eq!(home.out(&d2).phase, "created");
    assert_eq!(home.out(&d3).phase, "sealed");
    assert_eq!(b.requests().len(), 2, "{:?}", b.requests());

    // D1 kept its success, so the next tick polls it; D2 waits out its backoff.
    assert_eq!(home.tick(&mut follower), Vec::<String>::new());
    assert_eq!(b.count(&format!("poll {d1}")), 1);
    assert!(
        b.requests().iter().all(|seen| !seen.ends_with(&d2)),
        "{:?}",
        b.requests()
    );
}

#[test]
fn concurrent_results_for_one_owner_are_each_delivered_once() {
    let home = Home::new();
    install_runtime(
        &home.settings,
        InboxRuntime {
            agent_turn_backend: home.agent.clone(),
            runtime_handler_runner_factory: attractor_api::default_runtime_handler_runner_factory(),
            publish: Arc::new(|_| {}),
        },
    );
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    // Both peers answer at once, so both workers seal and deliver together,
    // while the owner's first turn is held open.
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let peer = |barrier: Arc<std::sync::Barrier>| {
        Peer::serve(move |_, id| {
            barrier.wait();
            (200, b_state(id, "sealed", "completed"))
        })
    };
    let (one, two) = (peer(barrier.clone()), peer(barrier));
    home.link_to("env-1", &one);
    home.link_to("env-2", &two);
    let ids = [
        home.start_on("env-1", "one", Some("amber-owner")),
        home.start_on("env-2", "two", Some("amber-owner")),
    ];

    let mut follower = delegations::Follower::default();
    assert_eq!(home.tick(&mut follower), Vec::<String>::new());
    for id in &ids {
        assert_eq!(home.out(id).phase, "sealed", "{id}");
        assert_eq!(home.results("chat-owner", id).len(), 1, "{id}");
    }
    home.agent.release(4);
    let delivered = || {
        let events = home.repository().inbox_events("chat-owner").unwrap();
        let head = home.repository().read_head("chat-owner", None).unwrap();
        let busy = head.unwrap()["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|turn| {
                matches!(
                    turn["status"].as_str(),
                    Some("pending" | "streaming" | "queued")
                )
            });
        events.len() == 2 && events.iter().all(|e| e.delivered_at.is_some()) && !busy
    };
    wait_for("both results delivered", delivered);
    home.tick(&mut follower);
    wait_for("both results delivered", delivered);
    let prompts = home.agent.prompts().join("\n");
    for id in &ids {
        let event = format!("[event delegation:{id}:result from {id}]");
        assert_eq!(prompts.matches(&event).count(), 1, "{prompts}");
    }
    assert_eq!(
        home.repository().inbox_events("chat-owner").unwrap().len(),
        2
    );
    assert_eq!(one.requests().len() + two.requests().len(), 2);
}

/// An answer one byte over the limit, its length declared or, when
/// `chunked`, streamed in chunks without one.
fn oversized_answer(chunked: bool) -> Vec<u8> {
    let limit = delegations::MAX_RESPONSE_BYTES as usize;
    let body = format!("{{\"pad\":\"{}\"}}", "x".repeat(limit - 9));
    assert_eq!(body.len(), limit + 1);
    let mut answer =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n".to_vec();
    if chunked {
        answer.extend(b"Transfer-Encoding: chunked\r\n\r\n");
        for chunk in body.as_bytes().chunks(64 * 1024) {
            answer.extend(format!("{:x}\r\n", chunk.len()).as_bytes());
            answer.extend(chunk);
            answer.extend(b"\r\n");
        }
        answer.extend(b"0\r\n\r\n");
    } else {
        answer.extend(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
    }
    answer
}

#[test]
fn a_follower_refuses_an_oversized_or_non_json_answer_and_keeps_the_last_state() {
    let home = Home::new();
    // 0: B's ordinary view; 1: an oversized one with its length; 2: one
    // streamed without a length; 3: a success that is not JSON.
    let mode = Arc::new(AtomicUsize::new(0));
    let answering = mode.clone();
    let b = Peer::serve_raw(move |_, id| match answering.load(Ordering::SeqCst) {
        0 => json_answer(200, &b_state(id, "launched", "running")),
        3 => b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\
               Connection: close\r\n\r\nok"
            .to_vec(),
        n => oversized_answer(n == 2),
    });
    home.link_to(LINKED, &b);
    let id = home.start_on(LINKED, "build", None);
    assert_eq!(
        home.tick(&mut delegations::Follower::default()),
        Vec::<String>::new()
    );
    let valid = home.out(&id);
    assert_eq!(valid.phase, "accepted");
    assert!(valid.last_state.is_some());

    let too_large = format!("more than {} bytes", delegations::MAX_RESPONSE_BYTES);
    for (n, refused) in [
        (1, too_large.as_str()),
        (2, &too_large),
        (3, "200 with a body that is not JSON"),
    ] {
        mode.store(n, Ordering::SeqCst);
        let diagnostics = home.tick(&mut delegations::Follower::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].contains(&id) && diagnostics[0].contains(refused),
            "{diagnostics:?}"
        );
        let kept = home.out(&id);
        assert_eq!(
            (&kept.phase, &kept.last_state, &kept.paused),
            (&valid.phase, &valid.last_state, &None),
            "answer {n}"
        );
    }
    assert_eq!(
        b.requests(),
        [
            format!("submit {id}"),
            format!("poll {id}"),
            format!("poll {id}"),
            format!("poll {id}")
        ]
    );
}

#[test]
fn a_follower_caps_outstanding_question_notifications_per_delegation() {
    let home = Home::new();
    let store = crate::shared_store(&home.settings.data_dir);
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    // B reports these questions open.
    let open = Arc::new(Mutex::new(Vec::<String>::new()));
    let asked = open.clone();
    let b = Peer::serve(move |_, id| {
        let mut state = b_state(id, "launched", "waiting");
        state["questions"] = asked
            .lock()
            .unwrap()
            .iter()
            .map(|question| json!({"question_id": question}))
            .collect();
        (200, state)
    });
    home.link_to(LINKED, &b);
    let id = home.start_on(LINKED, "ask", Some("amber-owner"));
    let ask = |questions: &[String]| {
        *open.lock().unwrap() = questions.to_vec();
        home.tick(&mut delegations::Follower::default())
    };
    let notified = || {
        home.repository()
            .inbox_events("chat-owner")
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "delegation.question")
            .count()
    };
    let refused = |diagnostics: Vec<String>| {
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].contains(&id) && diagnostics[0].contains("outstanding"),
            "{diagnostics:?}"
        );
    };
    let cap = delegations::MAX_OUTSTANDING_QUESTIONS;
    let fresh = |n: usize| vec![format!("q-{n}")];

    // B asks a fresh question on each poll and closes the last, while the
    // owner takes none of them: they stay outstanding up to the cap.
    for n in 0..cap {
        assert_eq!(ask(&fresh(n)), Vec::<String>::new(), "question {n}");
    }
    assert_eq!(notified(), cap);
    let held = home.out(&id).last_state;
    refused(ask(&fresh(cap)));
    assert_eq!(notified(), cap);
    assert_eq!(home.out(&id).last_state, held);
    assert!(home.out(&id).paused.is_none());

    // Once the owner has taken them, closed questions no longer count.
    home.agent.release(100);
    install_runtime(
        &home.settings,
        InboxRuntime {
            agent_turn_backend: home.agent.clone(),
            runtime_handler_runner_factory: attractor_api::default_runtime_handler_runner_factory(),
            publish: Arc::new(|_| {}),
        },
    );
    let taken = || {
        home.repository()
            .pending_inbox_events("chat-owner")
            .unwrap()
            .is_empty()
    };
    spark_workspace::inbox::process_conversation(&home.settings, &store, "chat-owner").unwrap();
    wait_for("the owner to take its questions", taken);
    assert_eq!(ask(&fresh(cap)), Vec::<String>::new());
    assert_eq!(notified(), cap + 1);
    wait_for("the owner to take its question", taken);

    // Open questions count however soon the owner takes them.
    let many: Vec<_> = (0..=cap).map(|n| format!("open-{n}")).collect();
    refused(ask(&many));
    assert_eq!(notified(), cap + 1);
    assert_eq!(ask(&many[..cap]), Vec::<String>::new());
    assert_eq!(notified(), 2 * cap + 1);
    let idle = || {
        let head = home.repository().read_head("chat-owner", None).unwrap();
        !head.unwrap()["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|turn| {
                matches!(
                    turn["status"].as_str(),
                    Some("pending" | "streaming" | "queued")
                )
            })
    };
    wait_for("the owner's turns to end", idle);
}

#[test]
fn a_refuses_answer_ids_that_would_leave_their_route() {
    let home = Home::new();
    let b = Peer::serve(|_, _| (200, json!({"answered": true})));
    home.link_to(LINKED, &b);
    let id = home.start_on(LINKED, "ask", None);
    assert!(home.environments().accept_out(&id, "run-1").unwrap());
    let answer = |delegation_id: &str, question_id: &str| {
        delegations::answer(
            &home.settings,
            delegation_id,
            question_id,
            serde_json::from_value(json!({"selected_value": "yes"})).unwrap(),
        )
    };
    for bad in ["", ".", "..", "a/../b", "a\\b", "a\nb"] {
        for refused in [answer(&id, bad), answer(bad, "q-1")] {
            assert!(
                matches!(&refused, Err(WorkspaceError::Validation(_))),
                "{bad:?}: {refused:?}"
            );
        }
    }
    assert!(b.requests().is_empty(), "{:?}", b.requests());
    assert_eq!(answer(&id, "q.1").unwrap(), json!({"answered": true}));
    assert_eq!(b.requests(), [format!("questions/q.1/answer {id}")]);
}

#[test]
fn sibling_child_run_questions_sharing_a_gate_id_notify_and_answer_independently() {
    let home = Home::new();
    let store = RunStore::for_settings(&home.settings);

    // Parent run with two sibling child runs paused on the exact same gate ID.
    let mut parent = RunRecord::new("run-parent", &home.project);
    parent.status = "running".into();
    store
        .create_run(CreateRunRequest {
            record: parent,
            ..Default::default()
        })
        .unwrap();

    for child_id in ["run-child-1", "run-child-2"] {
        let mut child = RunRecord::new(child_id, &home.project);
        child.status = "waiting".into();
        child.parent_run_id = Some("run-parent".into());
        child.root_run_id = Some("run-parent".into());
        let paths = store
            .create_run(CreateRunRequest {
                record: child,
                ..Default::default()
            })
            .unwrap();
        let gate = attractor_runtime::human_gate_pending_event(
            child_id,
            "gate-0",
            "review",
            "Review",
            "Approve?",
            None,
            vec![json!({"key": "1", "label": "Approve", "value": "approve"})],
        );
        store.append_event(&paths, gate).unwrap();
    }

    // An unrelated run outside the delegated run tree sharing the same gate ID.
    let mut unrelated = RunRecord::new("run-other", &home.project);
    unrelated.status = "waiting".into();
    let other_paths = store
        .create_run(CreateRunRequest {
            record: unrelated,
            ..Default::default()
        })
        .unwrap();
    let other_gate = attractor_runtime::human_gate_pending_event(
        "run-other",
        "gate-0",
        "review",
        "Review",
        "Approve?",
        None,
        vec![json!({"key": "1", "label": "Approve", "value": "approve"})],
    );
    store.append_event(&other_paths, other_gate).unwrap();

    let delegation_id = "00000000-0000-4000-8000-000000000099";
    home.launched(delegation_id, "run", "run-parent");
    let initial = home.state(delegation_id);
    let initial_runs: std::collections::BTreeSet<&str> = initial["questions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| q["run_id"].as_str())
        .collect();
    assert_eq!(
        initial_runs,
        std::collections::BTreeSet::from(["run-child-1", "run-child-2"])
    );

    // Omitting run_id when two children share gate-0 refuses to guess.
    let ambiguous = answer(
        &home,
        delegation_id,
        "gate-0",
        json!({"selected_value": "approve"}),
    );
    assert!(
        matches!(ambiguous, Err(WorkspaceError::Conflict(_))),
        "{ambiguous:?}"
    );

    // An out-of-tree run_id is rejected without touching either sibling.
    let out_of_tree = answer(
        &home,
        delegation_id,
        "gate-0",
        json!({"run_id": "run-other", "selected_value": "approve"}),
    );
    assert!(
        matches!(out_of_tree, Err(WorkspaceError::NotFound(_))),
        "{out_of_tree:?}"
    );

    // Naming run-child-2 answers only run-child-2 and leaves run-child-1 open.
    answer(
        &home,
        delegation_id,
        "gate-0",
        json!({"run_id": "run-child-2", "selected_value": "approve"}),
    )
    .unwrap();
    let remaining = home.state(delegation_id);
    let remaining_runs: Vec<&str> = remaining["questions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| q["run_id"].as_str())
        .collect();
    assert_eq!(remaining_runs, vec!["run-child-1"]);

    // On A's follower side, sibling child questions sharing gate-0 produce distinct inbox events.
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    let b = Peer::serve(move |_, id| {
        let mut state = b_state(id, "launched", "waiting");
        state["questions"] = json!([
            {"question_id": "gate-0", "run_id": "run-child-1"},
            {"question_id": "gate-0", "run_id": "run-child-2"},
        ]);
        (200, state)
    });
    home.link_to(LINKED, &b);
    let out_id = home.start_on(LINKED, "siblings", Some("amber-owner"));
    assert_eq!(
        home.tick(&mut delegations::Follower::default()),
        Vec::<String>::new()
    );
    let event_ids: std::collections::BTreeSet<String> = home
        .repository()
        .inbox_events("chat-owner")
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "delegation.question")
        .map(|event| event.id)
        .collect();
    assert_eq!(
        event_ids,
        std::collections::BTreeSet::from([
            format!("delegation:{out_id}:question:run-child-1:gate-0"),
            format!("delegation:{out_id}:question:run-child-2:gate-0"),
        ])
    );
}

#[test]
fn follower_seals_a_never_submitted_canceled_delegation_when_pausing() {
    let home = Home::new();
    home.seed_chat("chat-owner", "amber-owner", "chat", "complete", json!([]));
    // Peer rejects with 401 before any submission occurs.
    let b = Peer::serve(|_, _| (401, json!({"detail": "revoked"})));
    home.link_to(LINKED, &b);

    // Case 1: cancel requested, then follower hits 401 while pausing.
    let refused_id = home.start_on(LINKED, "cancel-before-401", Some("amber-owner"));
    home.environments().request_cancel_out(&refused_id).unwrap();
    assert_eq!(
        home.tick(&mut delegations::Follower::default()),
        Vec::<String>::new()
    );
    let refused = home.out(&refused_id);
    assert_eq!(refused.phase, "sealed");
    assert_eq!(refused.result.as_ref().unwrap()["status"], "canceled");

    // Case 2: cancel requested, then remove_link pauses the unsubmitted record.
    let unlinked_id = home.start_on(LINKED, "cancel-before-unlink", Some("amber-owner"));
    home.environments()
        .request_cancel_out(&unlinked_id)
        .unwrap();
    home.environments().remove_link(LINKED).unwrap();
    assert_eq!(
        home.tick(&mut delegations::Follower::default()),
        Vec::<String>::new()
    );
    let unlinked = home.out(&unlinked_id);
    assert_eq!(unlinked.phase, "sealed");
    assert_eq!(unlinked.result.as_ref().unwrap()["status"], "canceled");
    assert_eq!(home.results("chat-owner", &refused_id).len(), 1);
    assert_eq!(home.results("chat-owner", &unlinked_id).len(), 1);
}
