//! Whole-task handoff: the records A keeps of what it delegates, and how B
//! admits, launches, reports and seals a delegation, driven through the
//! workspace API with no network between them.
use super::mission_contracts::{project, wait_for, GatedAgent};
use super::review_artifact_contracts::{settings, simple_flow, write_native_execution_profile};
use attractor_core::{RunRecord, RunResult};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::environment::{DelegationIn, EnvironmentLink, EnvironmentRepository};
use spark_storage::{ConversationRepository, ProjectRegistry};
use spark_workspace::delegations::{
    self, request_digest, DelegationStartRequest, IncomingCancel, IncomingDelegation,
    DELEGATION_CONTEXT_KEY,
};
use spark_workspace::{WorkspaceConversationService, WorkspaceError};
use std::sync::Arc;

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
