use super::review_artifact_contracts::{
    seed_conversation, settings, simple_flow, write_flow, write_native_execution_profile,
};
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
    AGENT_INSTRUCTIONS_METADATA_KEY,
};
use spark_common::settings::SparkSettings;
use spark_workspace::{
    missions::{
        install_runtime, MissionEventPost, MissionMutation, MissionRecord, MissionRuntime,
        MissionStatus, WorkspaceMissionService,
    },
    FlowRunRequestCreateByHandleRequest, WorkspaceConversationService, WorkspaceError,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

fn mutation(value: Value) -> MissionMutation {
    serde_json::from_value(value).unwrap()
}
fn message(text: &str) -> MissionEventPost {
    serde_json::from_value(json!({"kind": "human.message", "payload": {"message": text}})).unwrap()
}
fn project(settings: &SparkSettings) -> String {
    std::fs::create_dir_all(&settings.project_root).unwrap();
    settings.project_root.to_str().unwrap().to_string()
}
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An agent whose turns finish only when the test opens its gate; it records
/// every request.
#[derive(Default)]
struct GatedAgent {
    requests: Mutex<Vec<AgentTurnRequest>>,
    gate: Mutex<usize>,
    opened: Condvar,
}
impl GatedAgent {
    /// Lets the next `count` turns finish.
    fn release(&self, count: usize) {
        *self.gate.lock().unwrap() += count;
        self.opened.notify_all();
    }
    fn prompts(&self) -> Vec<String> {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .map(|request| request.prompt.clone())
            .collect()
    }
}
impl AgentTurnBackend for GatedAgent {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.requests.lock().unwrap().push(request);
        let mut gate = self.gate.lock().unwrap();
        while *gate == 0 {
            gate = self.opened.wait(gate).unwrap();
        }
        *gate -= 1;
        Ok(final_answer("Noted."))
    }
}

struct Harness {
    _temp: tempfile::TempDir,
    settings: SparkSettings,
    project: String,
    agent: Arc<GatedAgent>,
    missions: WorkspaceMissionService,
}
impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let project = project(&settings);
        write_native_execution_profile(&settings);
        let agent = Arc::new(GatedAgent::default());
        install_runtime(
            &settings,
            MissionRuntime {
                agent_turn_backend: agent.clone(),
                publish: Arc::new(|_| {}),
            },
        );
        let missions = WorkspaceMissionService::new(settings.clone());
        Self {
            _temp: temp,
            settings,
            project,
            agent,
            missions,
        }
    }
    fn create(&self, fields: Value) -> MissionRecord {
        self.missions
            .create(&self.project, mutation(json!({ "fields": fields })))
            .unwrap()
    }
    fn get(&self, id: &str) -> MissionRecord {
        self.missions.get(&self.project, id).unwrap()
    }
    fn wait_turns(&self, count: usize) {
        wait_for(&format!("{count} turns"), || {
            self.agent.requests.lock().unwrap().len() >= count
        });
    }
    /// Waits until the mission has no turn in flight and nothing pending.
    fn wait_idle(&self, id: &str) {
        wait_for("an idle mission", || {
            self.get(id).status != MissionStatus::Running
        });
    }
    fn transcript(&self, id: &str) -> Value {
        WorkspaceConversationService::new(self.settings.clone())
            .get_snapshot(id, Some(&self.project))
            .unwrap()
    }
    fn handle(&self, id: &str) -> String {
        self.transcript(id)["conversation_handle"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn agent_requestable(&self, flow: &str, content: &str) {
        write_flow(&self.settings, flow, content);
        spark_storage::set_flow_launch_policy(&self.settings.config_dir, flow, "agent_requestable")
            .unwrap();
    }
    /// The agent's `spark convo run-request` from inside its turn.
    fn launch(&self, id: &str, flow: &str, summary: &str) -> Result<String, WorkspaceError> {
        WorkspaceConversationService::new(self.settings.clone())
            .create_flow_run_request_by_handle(
                &self.handle(id),
                FlowRunRequestCreateByHandleRequest {
                    flow_name: flow.into(),
                    summary: summary.into(),
                    execution_profile_id: Some("native".into()),
                    ..Default::default()
                },
            )
            .map(|response| response["run_id"].as_str().unwrap().to_string())
    }
    /// Waits for a run to finish and hands its state to the mission, as the
    /// server's terminal publish hook does.
    fn settle(&self, run_id: &str) {
        wait_terminal(&self.settings, run_id);
        self.missions.deliver_run_events(run_id).unwrap();
    }
}

fn wait_terminal(settings: &SparkSettings, run_id: &str) -> String {
    let store = attractor_runtime::RunStore::for_settings(settings);
    let mut status = String::new();
    wait_for(&format!("run {run_id}"), || {
        let paths = store.find_run_root(run_id).unwrap();
        status = paths
            .as_ref()
            .and_then(|paths| store.read_run_record(paths).unwrap())
            .map(|record| attractor_runtime::normalize_run_status(&record.status))
            .unwrap_or_default();
        let pending = paths
            .and_then(|paths| store.read_result(&paths).unwrap())
            .is_none_or(|result| result.state == "pending");
        ["completed", "failed", "canceled", "validation_error"].contains(&status.as_str())
            && !pending
    });
    status
}

fn mission_root(settings: &SparkSettings, project: &str) -> std::path::PathBuf {
    spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root
}

/// Seeds a run record the mission owns, with the pending result `create_run` writes.
fn owned_run(
    settings: &SparkSettings,
    project: &str,
    mission_id: &str,
    run_id: &str,
    status: &str,
) {
    spark_storage::workspace_missions::MissionRepository::new(&mission_root(settings, project))
        .transact::<WorkspaceError>(mission_id, |value| {
            let mut value = value.unwrap();
            value["runs"].as_array_mut().unwrap().push(json!({"run_id":run_id,"flow_name":"work/seeded.yaml","summary":"Seeded work","launched_at":"t","status":"running"}));
            Ok(value)
        })
        .unwrap();
    let mut record = attractor_core::RunRecord::new(run_id, project);
    record.status = status.into();
    record.launch_context = Some(
        [(
            "context.spark_mission".to_string(),
            json!({"mission_id": mission_id}),
        )]
        .into(),
    );
    let store = attractor_runtime::RunStore::for_settings(settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(&paths, &attractor_core::RunResult::pending(run_id, status))
        .unwrap();
}

fn tool_flow(command: &str) -> String {
    json!({
        "schema_version": "1",
        "id": "tool",
        "nodes": {
            "start": {"kind": "start"},
            "work": {"kind": "tool", "config": {"kind": "tool", "command": command}},
            "done": {"kind": "exit"}
        },
        "edges": [{"from": "start", "to": "work"}, {"from": "work", "to": "done", "condition": "outcome=success"}]
    })
    .to_string()
}

#[test]
fn legacy_records_load_and_unstarted_finished_cards_are_archived() {
    let harness = Harness::new();
    let root = mission_root(&harness.settings, &harness.project);
    let project_id = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .ensure_project_paths(&harness.project)
        .unwrap()
        .project_id;
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    for (id, stage, started) in [
        ("task-done", "done", Value::Null),
        ("task-review", "review", Value::Null),
        ("task-ready", "ready", Value::Null),
        ("task-running", "review", json!("2026-01-01")),
    ] {
        let legacy = json!({
            "id": id, "project_id": project_id, "project_path": harness.project,
            "created_at": format!("2026-01-01 {id}"), "updated_at": "2026-01-01", "revision": 1,
            "fields": {"title": id, "description": "Kept", "stage": stage, "archived": false,
                "reaction_flow": "missions/react.yaml", "hooks": [{"on": "run.completed", "do": "ignore"}],
                "budget": {"concurrent_runs": 2, "total_runs": 5, "reactions": 10}},
            "activity": [{"revision": 1, "actor": "human", "note": ""}],
            "state": "old state", "execution": {"substate": "attention", "reason": "held"},
            "paused": true, "hold": {"substate": "attention", "reason": "held", "actions": [], "event": ""},
            "pending_events": [], "reaction_events": [], "started_at": started,
            "runs": [{"run_id": "run-old", "label": "build", "role": "reaction", "launched_at": "t", "launched_by_event": "e", "status": "completed"}],
        });
        std::fs::write(root.join(format!("tasks/{id}.json")), legacy.to_string()).unwrap();
    }
    let missions = harness.missions.list(&harness.project).unwrap();
    assert!(root.join("missions/task-done.json").exists());
    let archived: Vec<_> = missions
        .iter()
        .map(|mission| (mission.id.as_str(), mission.fields.archived))
        .collect();
    assert_eq!(
        archived,
        [
            ("task-done", true),
            ("task-ready", false),
            ("task-review", true),
            ("task-running", false)
        ]
    );
    let ready = &missions[1];
    assert_eq!(ready.status, MissionStatus::Draft);
    assert_eq!(
        (
            ready.fields.budget.concurrent_runs,
            ready.fields.budget.total_runs
        ),
        (2, 5)
    );
    assert_eq!(ready.runs[0].run_id, "run-old");
    let updated = harness
        .missions
        .update(
            &harness.project,
            "task-done",
            mutation(json!({"revision": 1, "fields": {"title": "Renamed"}})),
        )
        .unwrap();
    // A write drops the removed fields and keeps the archive.
    let stored: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("missions/task-done.json")).unwrap(),
    )
    .unwrap();
    assert!(updated.fields.archived);
    for removed in [
        "state",
        "execution",
        "paused",
        "hold",
        "pending_events",
        "status",
    ] {
        assert!(stored.get(removed).is_none(), "{removed}");
    }
    assert!(stored["fields"].get("stage").is_none());
    assert!(stored["fields"].get("hooks").is_none());
}

#[test]
fn mutations_accept_only_editable_fields_and_reject_stale_revisions() {
    let harness = Harness::new();
    let mission = harness.create(json!({"title": "One", "description": "Ship"}));
    assert!(mission.id.starts_with("mission-"));
    assert_eq!(mission.status, MissionStatus::Draft);
    assert_eq!(
        json!(mission.fields.budget),
        json!({"concurrent_runs": 4, "total_runs": 25})
    );
    for key in [
        "stage",
        "hooks",
        "reaction_flow",
        "state",
        "runs",
        "closed",
        "status",
    ] {
        assert!(
            harness
                .missions
                .update(
                    &harness.project,
                    &mission.id,
                    mutation(json!({"revision": 1, "fields": {key: null}}))
                )
                .is_err(),
            "{key}"
        );
    }
    assert!(serde_json::from_value::<MissionMutation>(json!({"yaml": "budget: {}"})).is_err());
    let raised = harness
        .missions
        .update(
            &harness.project,
            &mission.id,
            mutation(json!({"revision": 1, "fields": {"budget": {"total_runs": 3}}})),
        )
        .unwrap();
    assert_eq!(
        (
            raised.fields.budget.concurrent_runs,
            raised.fields.budget.total_runs
        ),
        (4, 3)
    );
    assert!(matches!(
        harness.missions.update(
            &harness.project,
            &mission.id,
            mutation(json!({"revision": 1, "fields": {"title": "Stale"}}))
        ),
        Err(WorkspaceError::Conflict(_))
    ));
}

#[test]
fn start_creates_the_conversation_and_pins_the_objective_outside_the_transcript() {
    let harness = Harness::new();
    let mission =
        harness.create(json!({"title": "Search", "description": "Search returns documents."}));
    let started = harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    assert_eq!(
        started.conversation_id.as_deref(),
        Some(mission.id.as_str())
    );
    assert_eq!(started.status, MissionStatus::Running);
    assert!(matches!(
        harness.missions.start(&harness.project, &mission.id),
        Err(WorkspaceError::Conflict(_))
    ));
    harness.wait_turns(1);
    let request = harness.agent.requests.lock().unwrap()[0].clone();
    assert_eq!(
        request.prompt,
        "Objective:\nSearch returns documents.\n\nBegin work on this mission."
    );
    let frame = request.metadata[AGENT_INSTRUCTIONS_METADATA_KEY]
        .as_str()
        .unwrap();
    assert!(frame.contains("Search returns documents."));
    assert!(frame.contains(&format!("Mission ID: {}", mission.id)));
    assert!(frame.contains("spark mission close"));
    assert!(!frame.contains("approve"));
    // The mission's thread stays out of the project's Threads list.
    let threads = WorkspaceConversationService::new(harness.settings.clone())
        .list_project_conversations(&harness.project)
        .unwrap();
    assert!(threads
        .iter()
        .all(|thread| thread.conversation_id != mission.id));
    harness.agent.release(1);
    harness.wait_idle(&mission.id);
    assert_eq!(harness.get(&mission.id).status, MissionStatus::NeedsYou);
    let turns = harness.transcript(&mission.id)["turns"].clone();
    assert_eq!(turns.as_array().unwrap().len(), 2);
    assert_eq!(turns[1]["status"], "complete", "{turns}");
    assert!(!turns.to_string().contains("Spark control surface"));
}

#[test]
fn events_during_a_turn_are_delivered_as_one_next_turn() {
    let harness = Harness::new();
    harness.agent_requestable("work/ok.yaml", simple_flow());
    harness.agent_requestable("work/fail.yaml", &tool_flow("exit 1"));
    let mission = harness.create(json!({"title": "Ship", "description": "Deliver"}));
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    // The agent launches two runs during its first turn.
    let first = harness
        .launch(&mission.id, "work/ok.yaml", "Build it")
        .unwrap();
    let second = harness
        .launch(&mission.id, "work/fail.yaml", "Break it")
        .unwrap();
    let launched = harness.get(&mission.id);
    let roster: Vec<_> = launched
        .runs
        .iter()
        .map(|run| {
            (
                run.run_id.as_str(),
                run.flow_name.as_str(),
                run.summary.as_str(),
            )
        })
        .collect();
    assert_eq!(
        roster,
        [
            (first.as_str(), "work/ok.yaml", "Build it"),
            (second.as_str(), "work/fail.yaml", "Break it")
        ]
    );
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store.find_run_root(&first).unwrap().unwrap();
    let context = store
        .read_run_record(&paths)
        .unwrap()
        .unwrap()
        .launch_context;
    assert_eq!(
        json!(context)["context.spark_mission"],
        json!({"mission_id": mission.id})
    );
    harness.settle(&first);
    harness.settle(&second);
    // A typed message waits too: at most one turn is in flight.
    harness
        .missions
        .post_event(
            &harness.project,
            &mission.id,
            message("Prefer the smaller fix"),
        )
        .unwrap();
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 1);
    assert_eq!(harness.get(&mission.id).status, MissionStatus::Running);
    harness.agent.release(2);
    harness.wait_turns(2);
    harness.wait_idle(&mission.id);
    let prompts = harness.agent.prompts();
    assert_eq!(prompts.len(), 2);
    let lines: Vec<_> = prompts[1].split("\n\n").collect();
    assert_eq!(lines.len(), 3, "{}", prompts[1]);
    assert_eq!(
        lines[0],
        format!("Run {first} (work/ok.yaml, \"Build it\") ended completed.")
    );
    assert!(lines[1].starts_with(&format!(
        "Run {second} (work/fail.yaml, \"Break it\") ended failed"
    )));
    assert_eq!(lines[2], "User: Prefer the smaller fix");
    let mission = harness.get(&mission.id);
    assert_eq!(mission.cursor, mission.event_seq);
    assert_eq!(mission.status, MissionStatus::NeedsYou);
    // The transcript shows both launches inline.
    assert_eq!(
        harness.transcript(&mission.id)["flow_launches"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn launches_over_budget_or_outside_the_catalog_are_refused() {
    let harness = Harness::new();
    harness.agent_requestable("work/slow.yaml", &tool_flow("sleep 30"));
    write_flow(&harness.settings, "work/private.yaml", simple_flow());
    let mission = harness
        .create(json!({"title": "Budgeted", "budget": {"concurrent_runs": 1, "total_runs": 2}}));
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    let refused = harness
        .launch(&mission.id, "work/private.yaml", "Nope")
        .unwrap_err();
    assert!(
        refused.to_string().contains("not agent-requestable"),
        "{refused}"
    );
    harness
        .launch(&mission.id, "work/slow.yaml", "One")
        .unwrap();
    let refused = harness
        .launch(&mission.id, "work/slow.yaml", "Two")
        .unwrap_err();
    assert!(
        refused.to_string().contains("concurrent_runs (1)"),
        "{refused}"
    );
    assert_eq!(harness.get(&mission.id).runs.len(), 1);
    let canceled = harness
        .missions
        .cancel(&harness.project, &mission.id)
        .unwrap();
    harness.agent.release(1);
    assert_eq!(canceled.status, MissionStatus::Closed);
    let refused = harness
        .launch(&mission.id, "work/slow.yaml", "Three")
        .unwrap_err();
    assert!(refused.to_string().contains("closed"), "{refused}");
}

#[test]
fn cancel_stops_owned_runs_and_closes_the_mission() {
    let harness = Harness::new();
    harness.agent_requestable("work/slow.yaml", &tool_flow("sleep 3"));
    // The canceled run finishes once its short command exits.
    let mission = harness.create(json!({"title": "Stop"}));
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    let run = harness
        .launch(&mission.id, "work/slow.yaml", "Long")
        .unwrap();
    harness.agent.release(1);
    let canceled = harness
        .missions
        .cancel(&harness.project, &mission.id)
        .unwrap();
    let closed = canceled.closed.as_ref().unwrap();
    assert_eq!(
        (json!(closed.status), closed.actor.as_str()),
        (json!("canceled"), "human")
    );
    assert_eq!(wait_terminal(&harness.settings, &run), "canceled");
    harness.missions.deliver_run_events(&run).unwrap();
    let mission = harness.get(&mission.id);
    assert_eq!(mission.runs[0].status, "canceled");
    assert_eq!(mission.status, MissionStatus::Closed);
    // Nothing more is delivered once closed.
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 1);
}

#[test]
fn the_agent_closes_the_mission_and_the_close_joins_the_transcript() {
    let harness = Harness::new();
    let mission = harness.create(json!({"title": "Close me"}));
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    let close = serde_json::from_value(
        json!({"status": "done", "reason": "Objective met", "actor": "assistant"}),
    )
    .unwrap();
    let closed = harness
        .missions
        .close(&harness.project, &mission.id, close)
        .unwrap();
    harness.agent.release(1);
    let record = closed.closed.as_ref().unwrap();
    assert_eq!(
        (record.reason.as_str(), record.actor.as_str()),
        ("Objective met", "assistant")
    );
    assert_eq!(closed.status, MissionStatus::Closed);
    wait_for("the turn to finish", || {
        !harness.transcript(&mission.id)["turns"]
            .to_string()
            .contains("\"pending\"")
    });
    let turns = harness.transcript(&mission.id)["turns"].clone();
    let notice = turns
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["kind"] == "mission_notice")
        .unwrap();
    assert_eq!(notice["content"], "Closed as done: Objective met");
}

#[test]
fn derived_status_covers_each_group_including_a_run_waiting_on_a_gate() {
    let harness = Harness::new();
    let mission = harness.create(json!({"title": "Status"}));
    assert_eq!(mission.status, MissionStatus::Draft);
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    assert_eq!(harness.get(&mission.id).status, MissionStatus::Running);
    harness.agent.release(1);
    harness.wait_idle(&mission.id);
    assert_eq!(harness.get(&mission.id).status, MissionStatus::NeedsYou);
    owned_run(
        &harness.settings,
        &harness.project,
        &mission.id,
        "run-gate",
        "running",
    );
    assert_eq!(harness.get(&mission.id).status, MissionStatus::Running);
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store.find_run_root("run-gate").unwrap().unwrap();
    let mut record = store.read_run_record(&paths).unwrap().unwrap();
    record.status = "waiting".into();
    store.write_run_record(&paths, &record).unwrap();
    harness.missions.deliver_run_events("run-gate").unwrap();
    harness.wait_turns(2);
    // A gate needs a human even while the agent is working on its news.
    assert_eq!(harness.get(&mission.id).status, MissionStatus::NeedsYou);
    assert!(harness.agent.prompts()[1].contains("is waiting on a human gate"));
    let attention = WorkspaceConversationService::new(harness.settings.clone())
        .pending_attention()
        .unwrap();
    assert!(attention
        .iter()
        .any(|item| item["kind"] == "mission" && item["id"] == json!(mission.id)));
    harness.agent.release(1);
    let closed = harness
        .missions
        .cancel(&harness.project, &mission.id)
        .unwrap();
    assert_eq!(closed.status, MissionStatus::Closed);
}

#[test]
fn recovery_fails_stale_turns_and_delivers_missing_terminal_events() {
    let harness = Harness::new();
    let mission = harness.create(json!({"title": "Recover"}));
    harness
        .missions
        .start(&harness.project, &mission.id)
        .unwrap();
    harness.wait_turns(1);
    harness.agent.release(1);
    harness.wait_idle(&mission.id);
    // A run failed before a restart while its result was still pending.
    owned_run(
        &harness.settings,
        &harness.project,
        &mission.id,
        "run-lost",
        "failed",
    );
    assert!(harness
        .missions
        .deliver_run_events("run-lost")
        .unwrap()
        .is_none());
    harness.agent.release(1);
    let recovered = harness.missions.recover().unwrap();
    assert_eq!(recovered.len(), 1);
    harness.wait_turns(2);
    assert_eq!(
        harness.agent.prompts()[1],
        "Run run-lost (work/seeded.yaml, \"Seeded work\") ended failed."
    );
    harness.wait_idle(&mission.id);
    let mission = harness.get(&mission.id);
    assert_eq!(mission.runs[0].status, "failed");
    assert_eq!(mission.status, MissionStatus::NeedsYou);
    // A second recovery finds nothing new to deliver.
    harness.missions.recover().unwrap();
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 2);
}

#[test]
fn missions_stay_inside_their_project() {
    let harness = Harness::new();
    let mission = harness.create(json!({"title": "Scoped"}));
    let other = harness._temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let other = other.to_str().unwrap();
    assert!(harness.missions.get(other, &mission.id).is_err());
    assert!(harness
        .missions
        .get(&harness.project, "../outside")
        .is_err());
    assert!(harness.missions.list(other).unwrap().is_empty());
    assert!(harness
        .missions
        .post_event(other, &mission.id, message("hi"))
        .is_err());
    assert!(harness.missions.start(other, &mission.id).is_err());
    // Only typed messages may be posted.
    let spoofed = serde_json::from_value(json!({"kind": "run.completed", "payload": {}})).unwrap();
    assert!(harness
        .missions
        .post_event(&harness.project, &mission.id, spoofed)
        .is_err());
}

#[test]
fn ordinary_conversations_launch_directly_without_mission_integration() {
    let harness = Harness::new();
    harness.agent_requestable("ops/review.yaml", simple_flow());
    seed_conversation(&harness.settings, &harness.project, "conversation-task");
    let mission = harness.create(json!({"title": "Unrelated"}));
    let response = WorkspaceConversationService::new(harness.settings.clone())
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.yaml".into(),
                summary: "Execute work".into(),
                execution_profile_id: Some("native".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let run_id = response["run_id"].as_str().unwrap();
    assert_eq!(response["conversation_id"], "conversation-task");
    wait_terminal(&harness.settings, run_id);
    assert!(harness
        .missions
        .deliver_run_events(run_id)
        .unwrap()
        .is_none());
    assert_eq!(harness.get(&mission.id).revision, 1);
}

/// A final answer as a codex turn reports it.
fn final_answer(text: &str) -> spark_agent_adapter::AgentTurnOutput {
    static ITEMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let item = ITEMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut event = spark_common::events::TurnStreamEvent::content_delta(
        spark_common::events::TurnStreamChannel::Assistant,
        text,
    );
    event.kind = spark_common::events::TurnStreamEventKind::ContentCompleted;
    event.phase = Some("final_answer".into());
    event.source.app_turn_id = Some(format!("app-turn-{item}"));
    event.source.item_id = Some(format!("item-{item}"));
    spark_agent_adapter::AgentTurnOutput {
        events: vec![event],
        final_assistant_text: Some(text.into()),
        ..Default::default()
    }
}
