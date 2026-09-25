use super::review_artifact_contracts::{
    seed_conversation, settings, simple_flow, write_flow, write_native_execution_profile,
};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_workspace::{
    missions::{
        MissionEventPost, MissionMutation, MissionRecord, Role, Stage, Substate,
        WorkspaceMissionService,
    },
    FlowRunRequestCreateByHandleRequest, FlowRunRequestReviewRequest, WorkspaceConversationService,
    WorkspaceError,
};
use std::time::{Duration, Instant};

fn mutation(value: Value) -> MissionMutation {
    serde_json::from_value(value).unwrap()
}
fn post(kind: &str, id: Option<&str>) -> MissionEventPost {
    serde_json::from_value(json!({"kind": kind, "id": id, "payload": {"message": "hi"}})).unwrap()
}
fn project(settings: &SparkSettings) -> String {
    std::fs::create_dir_all(&settings.project_root).unwrap();
    settings.project_root.to_str().unwrap().to_string()
}

/// A tool flow whose single node prints `stdout`, optionally mapping JSON
/// output fields into context keys.
fn tool_flow(command: &str, output_map: Value, env_map: Value) -> String {
    let writes: Vec<_> = output_map.as_object().unwrap().keys().cloned().collect();
    json!({
        "schema_version": "1",
        "id": "tool",
        "nodes": {
            "start": {"kind": "start"},
            "work": {"kind": "tool", "config": {"kind": "tool", "command": command, "output_map": output_map, "env_map": env_map}, "contracts": {"writes_context": writes}},
            "done": {"kind": "exit"}
        },
        "edges": [{"from": "start", "to": "work"}, {"from": "work", "to": "done", "condition": "outcome=success"}]
    })
    .to_string()
}
fn directive_flow(settings: &SparkSettings, name: &str, actions: Value) {
    let output = json!({"d": {"actions": actions}}).to_string();
    write_flow(
        settings,
        name,
        &tool_flow(
            &format!("printf '%s' '{output}'"),
            json!({"context.mission.directive": "d"}),
            json!({}),
        ),
    );
}
fn wait_terminal(settings: &SparkSettings, run_id: &str) -> String {
    let store = attractor_runtime::RunStore::for_settings(settings);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = store
            .find_run_root(run_id)
            .unwrap()
            .and_then(|paths| store.read_run_record(&paths).unwrap())
            .map(|record| attractor_runtime::normalize_run_status(&record.status))
            .unwrap_or_default();
        let pending = store
            .find_run_root(run_id)
            .unwrap()
            .and_then(|paths| store.read_result(&paths).unwrap())
            .is_none_or(|result| result.state == "pending");
        if ["completed", "failed", "canceled", "validation_error"].contains(&status.as_str())
            && !pending
        {
            return status;
        }
        assert!(Instant::now() < deadline, "run {run_id} stuck at {status}");
        std::thread::sleep(Duration::from_millis(25));
    }
}
/// Waits for a run to finish and hands its state to the mission, as the
/// server's terminal publish hook does.
fn settle_run(service: &WorkspaceMissionService, settings: &SparkSettings, run_id: &str) {
    wait_terminal(settings, run_id);
    service.deliver_run_events(run_id).unwrap();
}
fn run_of<'a>(mission: &'a MissionRecord, label: &str) -> &'a str {
    &mission
        .runs
        .iter()
        .rev()
        .find(|run| run.label == label)
        .unwrap_or_else(|| panic!("no {label} run in {:?}", mission.runs))
        .run_id
}
fn launch_context(settings: &SparkSettings, run_id: &str) -> Value {
    let store = attractor_runtime::RunStore::for_settings(settings);
    let paths = store.find_run_root(run_id).unwrap().unwrap();
    json!(
        store
            .read_run_record(&paths)
            .unwrap()
            .unwrap()
            .launch_context
    )
}
fn kinds(service: &WorkspaceMissionService, project: &str, id: &str) -> Vec<String> {
    service
        .events(project, id, 0)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

#[test]
fn missions_persist_atomically_reject_stale_writers_and_support_manual_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    assert_eq!(registry.read_project_record(project).unwrap(), None);
    let mut mission = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Manual delivery"}})),
        )
        .unwrap();
    assert!(mission.id.starts_with("mission-"));
    assert_eq!(mission.fields.stage, Stage::Backlog);
    assert_eq!(mission.fields.reaction_flow, "missions/react.yaml");
    assert_eq!(
        json!(mission.fields.budget),
        json!({"concurrent_runs":4,"total_runs":25,"reactions":10})
    );
    for stage in ["planning", "ready", "in_progress", "review", "done"] {
        mission = service
            .update(
                project,
                &mission.id,
                mutation(json!({"revision":mission.revision,"fields":{"stage":stage}})),
            )
            .unwrap();
    }
    // Moving a card launches nothing.
    assert!(mission.runs.is_empty() && mission.started_at.is_none());
    assert!(service.events(project, &mission.id, 0).unwrap().is_empty());
    mission = service
        .update(
            project,
            &mission.id,
            mutation(
                json!({"revision":mission.revision,"fields":{"stage":"planning","archived":true}}),
            ),
        )
        .unwrap();
    assert!(mission.fields.archived);
    assert_eq!(mission.activity.len() as u64, mission.revision);
    let before = json!(mission);
    let outcomes = std::thread::scope(|scope| {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|title| {
                let (barrier, settings, mission) =
                    (barrier.clone(), settings.clone(), mission.clone());
                scope.spawn(move || {
                    barrier.wait();
                    WorkspaceMissionService::new(settings).update(
                        project,
                        &mission.id,
                        mutation(json!({"revision":mission.revision,"fields":{"title":title}})),
                    )
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .any(|r| matches!(r, Err(WorkspaceError::Conflict(_)))));
    let latest = service.get(project, &mission.id).unwrap();
    assert_eq!(latest.revision, mission.revision + 1);
    assert_eq!(latest.activity.last().unwrap()["before"], before["fields"]);
}

#[test]
fn mission_records_load_existing_task_files_as_a_superset() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(&project)
        .unwrap();
    let task = json!({
        "id": "task-legacy", "project_id": root.project_id, "project_path": root.project_path,
        "created_at": "2026-01-01", "updated_at": "2026-01-01", "revision": 1,
        "fields": {"title": "Legacy", "description": "Kept", "stage": "ready", "archived": false},
        "activity": [{"revision": 1, "actor": "human", "note": ""}]
    });
    std::fs::create_dir_all(root.root.join("tasks")).unwrap();
    std::fs::write(root.root.join("tasks/task-legacy.json"), task.to_string()).unwrap();
    let service = WorkspaceMissionService::new(settings.clone());
    let missions = service.list(&project).unwrap();
    assert!(root.root.join("missions/task-legacy.json").exists());
    assert!(!root.root.join("tasks").exists());
    assert_eq!(missions.len(), 1);
    let legacy = &missions[0];
    assert_eq!(
        (legacy.fields.title.as_str(), legacy.fields.stage),
        ("Legacy", Stage::Ready)
    );
    assert_eq!(legacy.fields.reaction_flow, "missions/react.yaml");
    assert_eq!(legacy.execution.substate, Substate::Idle);
    let updated = service
        .update(
            &project,
            "task-legacy",
            mutation(json!({"revision":1,"fields":{"stage":"planning"}})),
        )
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.activity.len(), 2);
}

#[test]
fn missions_reject_system_fields_and_preserve_scope_and_attribution() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service.create(project, mutation(json!({"fields":{"title":"One","stage":"done"},"actor":"assistant","note":"Useful detail"}))).unwrap();
    assert_eq!(mission.activity[0]["actor"], "assistant");
    assert_eq!(mission.activity[0]["note"], "Useful detail");
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let other = other.to_str().unwrap();
    assert!(service.get(other, &mission.id).is_err());
    assert!(service.get(project, "../outside").is_err());
    assert!(service.list(other).unwrap().is_empty());
    assert!(service
        .post_event(other, &mission.id, post("human.message", None))
        .is_err());
    // Only reactions write state; the roster and inbox cursor are system-owned.
    for key in [
        "state",
        "runs",
        "execution",
        "cursor",
        "closed",
        "priority",
        "conversations",
    ] {
        assert!(
            service
                .update(
                    project,
                    &mission.id,
                    mutation(json!({"revision":1,"fields":{key:null}}))
                )
                .is_err(),
            "{key}"
        );
    }
    assert!(service
        .update(
            project,
            &mission.id,
            mutation(json!({"revision":1,"fields":{"hooks":[{"on":"run.completed","do":{"set_state":{"markdown":"no"}}}]}}))
        )
        .is_err());
    for fields in [json!({"title":"  "}), json!({"stage":"unknown"})] {
        assert!(service
            .create(project, mutation(json!({"fields":fields})))
            .is_err());
    }
    let edited = service
        .update(
            project,
            &mission.id,
            mutation(json!({"revision":1,"yaml":"hooks:\n  - on: run.completed\n    label: build\n    do: ignore\nbudget:\n  total_runs: 3\n"})),
        )
        .unwrap();
    assert_eq!(edited.fields.hooks.len(), 1);
    assert_eq!(edited.fields.budget.total_runs, 3);
    assert_eq!(edited.fields.budget.concurrent_runs, 4);
    let second = service
        .create(project, mutation(json!({"fields":{"title":"Two"}})))
        .unwrap();
    assert_eq!(
        service.board(project).unwrap(),
        json!({"missions":[edited, second]})
    );
}

#[test]
fn inbox_delivery_is_idempotent_and_hooks_match_in_order_with_reason_default() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    directive_flow(&settings, "missions/react.yaml", json!([]));
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Hooks","hooks":[
                {"on":"human.message","do":"ignore"},
                {"on":"human.message","do":"reason"},
                {"on":"mission.started","do":"ignore"}
            ]}})),
        )
        .unwrap();
    // Unstarted missions queue events without processing them.
    let queued = service
        .post_event(project, &mission.id, post("human.message", Some("m1")))
        .unwrap();
    assert_eq!(queued.cursor, 0);
    let again = service
        .post_event(project, &mission.id, post("human.message", Some("m1")))
        .unwrap();
    assert_eq!(service.events(project, &mission.id, 0).unwrap().len(), 1);
    assert_eq!(again.cursor, 0);
    assert!(service
        .post_event(project, &mission.id, post("Bad Kind", None))
        .is_err());
    let started = service.start(project, &mission.id).unwrap();
    assert_eq!(started.fields.stage, Stage::InProgress);
    assert!(matches!(
        service.start(project, &mission.id),
        Err(WorkspaceError::Conflict(_))
    ));
    // The first matching hook (ignore) wins for both events: no reaction.
    assert_eq!(started.cursor, 2);
    assert!(started.runs.is_empty());
    assert!(started.pending_events.is_empty());
    let tail = service.events(project, &mission.id, 1).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].kind, "mission.started");
    // Without a matching hook an event goes to a reaction.
    let reasoned = service
        .post_event(project, &mission.id, post("run.signal", None))
        .unwrap();
    assert_eq!(reasoned.cursor, 3);
    assert_eq!(reasoned.runs.len(), 1);
    assert_eq!(reasoned.runs[0].role, Role::Reaction);
    assert_eq!(reasoned.execution.substate, Substate::Reasoning);
    assert_eq!(reasoned.reaction_events[0].kind, "run.signal");
    let context = launch_context(&settings, &reasoned.runs[0].run_id);
    assert_eq!(
        context["context.spark_mission"],
        json!({"mission_id": mission.id, "label": "reaction", "role": "reaction"})
    );
    assert_eq!(context["context.mission"]["title"], "Hooks");
    assert_eq!(
        context["context.mission"]["events"][0]["kind"],
        "run.signal"
    );
}

#[test]
fn reactions_run_one_at_a_time_batch_events_and_apply_directives_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    directive_flow(
        &settings,
        "missions/react.yaml",
        json!([{"set_state":{"markdown":"first"}},{"set_state":{"markdown":"second"}}]),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Batch"}})))
        .unwrap();
    let started = service.start(project, &mission.id).unwrap();
    let first = started.runs[0].run_id.clone();
    wait_terminal(&settings, &first);
    // The first reaction has not been delivered yet, so these two queue.
    service
        .post_event(project, &mission.id, post("human.message", None))
        .unwrap();
    let queued = service
        .post_event(project, &mission.id, post("human.message", None))
        .unwrap();
    assert_eq!(queued.runs.len(), 1);
    assert_eq!(queued.pending_events.len(), 2);
    // The next reaction will fail.
    write_flow(
        &settings,
        "missions/react.yaml",
        &tool_flow("exit 3", json!({}), json!({})),
    );
    service.deliver_run_events(&first).unwrap();
    let mission = service.get(project, &mission.id).unwrap();
    // Directive actions applied in order; reactions are the only writer of state.
    assert_eq!(mission.state, "second");
    assert_eq!(mission.runs.len(), 2);
    let second = &mission.runs[1];
    assert_eq!(second.role, Role::Reaction);
    let batch = &launch_context(&settings, &second.run_id)["context.mission"];
    assert_eq!(batch["events"].as_array().unwrap().len(), 2);
    assert_eq!(batch["state"], "second");
    // A failed reaction needs attention, keeps its batch, and is not retried.
    settle_run(&service, &settings, &second.run_id.clone());
    let mission = service.get(project, &mission.id).unwrap();
    assert_eq!(mission.runs.len(), 2);
    assert_eq!(mission.execution.substate, Substate::Attention);
    assert_eq!(mission.pending_events.len(), 2);
    // A human message clears attention and reacts with the accumulated batch.
    let retried = service
        .post_event(project, &mission.id, post("human.message", None))
        .unwrap();
    assert_eq!(retried.runs.len(), 3);
    assert_eq!(retried.reaction_events.len(), 3);
    settle_run(&service, &settings, &retried.runs[2].run_id.clone());
    let resumed = service.resume(project, &mission.id).unwrap();
    assert_eq!(resumed.runs.len(), 4, "Resume triggers the next reaction");
}

#[test]
fn budgets_refuse_launches_until_raised_and_cancel_stops_owned_runs() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    write_flow(
        &settings,
        "work/slow.yaml",
        &tool_flow("sleep 5", json!({}), json!({})),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Budget","budget":{"total_runs":1},"hooks":[
            {"on":"mission.started","do":{"launch":{"flow_name":"work/slow.yaml","label":"one"}}},
            {"on":"human.message","do":{"launch":{"flow_name":"work/slow.yaml","label":"two","context":{"context.request.note":"x"}}}}
        ]}})))
        .unwrap();
    let started = service.start(project, &mission.id).unwrap();
    assert_eq!(started.runs.len(), 1);
    assert_eq!(started.execution.substate, Substate::Running);
    assert_eq!(
        launch_context(&settings, &started.runs[0].run_id)["context.spark_mission"]["role"],
        "work"
    );
    let refused = service
        .post_event(project, &mission.id, post("human.message", None))
        .unwrap();
    assert_eq!(refused.runs.len(), 1);
    assert_eq!(refused.execution.substate, Substate::Waiting);
    assert!(refused.execution.reason.contains("total_runs (1)"));
    // Later events queue behind the refused launch.
    let held = service
        .post_event(project, &mission.id, post("run.signal", None))
        .unwrap();
    assert_eq!(held.cursor, refused.cursor);
    let raised = service
        .update(
            project,
            &mission.id,
            mutation(json!({"revision":held.revision,"fields":{"budget":{"total_runs":5,"reactions":0}}})),
        )
        .unwrap();
    assert_eq!(raised.runs.len(), 2);
    assert_eq!(raised.runs[1].label, "two");
    assert_eq!(
        launch_context(&settings, &raised.runs[1].run_id)["context.request.note"],
        "x"
    );
    // The queued signal falls to reason; the reaction budget refuses it.
    assert_eq!(raised.execution.substate, Substate::Waiting);
    assert!(raised.execution.reason.contains("reactions (0)"));
    let canceled = service.cancel(project, &mission.id).unwrap();
    assert_eq!(
        canceled.closed.as_ref().map(|c| c.status),
        Some(spark_workspace::missions::CloseStatus::Canceled)
    );
    assert_eq!(canceled.fields.stage, Stage::InProgress);
    // A human cancel needs no further attention.
    assert_eq!(canceled.execution.substate, Substate::Idle);
    assert_eq!(canceled.execution.reason, "Canceled by human");
    assert!(!in_attention(&settings, &mission.id));
    for run in &canceled.runs {
        assert_eq!(wait_terminal(&settings, &run.run_id), "canceled");
    }
}

fn in_attention(settings: &SparkSettings, id: &str) -> bool {
    WorkspaceConversationService::new(settings.clone())
        .pending_attention()
        .unwrap()
        .iter()
        .any(|item| item["mission_id"] == id)
}

#[test]
fn reaction_or_hook_failures_stay_in_attention_until_moved_to_done() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Fail","hooks":[
                {"on":"mission.started","do":"ignore"},
                {"on":"check.failed","do":{"close":{"status":"failed","reason":"check failed"}}}
            ]}})),
        )
        .unwrap();
    service.start(project, &mission.id).unwrap();
    let closed = service
        .post_event(project, &mission.id, post("check.failed", None))
        .unwrap();
    assert_eq!(closed.execution.substate, Substate::Attention);
    assert_eq!(closed.execution.reason, "check failed");
    assert!(in_attention(&settings, &mission.id));
    service
        .update(
            project,
            &mission.id,
            mutation(json!({"revision":closed.revision,"fields":{"stage":"done"}})),
        )
        .unwrap();
    assert!(!in_attention(&settings, &mission.id));
}

#[test]
fn a_failed_directive_launch_keeps_its_remaining_actions_until_resume() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    directive_flow(
        &settings,
        "missions/react.yaml",
        json!([
            {"launch":{"flow_name":"work/missing.yaml","label":"gone"}},
            {"set_state":{"markdown":"after"}},
            {"close":{"status":"done","reason":"finished"}}
        ]),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Directive"}})))
        .unwrap();
    let started = service.start(project, &mission.id).unwrap();
    settle_run(&service, &settings, &started.runs[0].run_id.clone());
    let held = service.get(project, &mission.id).unwrap();
    assert_eq!(held.execution.substate, Substate::Attention);
    assert!(held.execution.reason.contains("Launch of gone failed"));
    assert_eq!(held.state, "");
    assert!(held.closed.is_none());
    let resumed = service.resume(project, &mission.id).unwrap();
    assert_eq!(resumed.runs.len(), 1, "the failed launch is not retried");
    assert_eq!(resumed.state, "after");
    assert_eq!(resumed.closed.as_ref().unwrap().reason, "finished");
}

#[test]
fn a_budget_wait_never_replaces_attention_and_runs_after_it_clears() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    write_flow(
        &settings,
        "work/slow.yaml",
        &tool_flow("sleep 2", json!({}), json!({})),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Held","budget":{"concurrent_runs":1},"hooks":[
            {"on":"mission.started","do":{"launch":{"flow_name":"work/missing.yaml","label":"bad"}}},
            {"on":"x.one","do":{"launch":{"flow_name":"work/slow.yaml","label":"one"}}},
            {"on":"x.two","do":{"launch":{"flow_name":"work/slow.yaml","label":"two"}}},
            {"on":"run.completed","do":"ignore"}
        ]}})))
        .unwrap();
    let started = service.start(project, &mission.id).unwrap();
    assert_eq!(started.execution.substate, Substate::Attention);
    let one = service
        .post_event(project, &mission.id, post("x.one", None))
        .unwrap();
    assert_eq!(one.runs.len(), 1);
    let deferred = service
        .post_event(project, &mission.id, post("x.two", None))
        .unwrap();
    assert_eq!(deferred.runs.len(), 1);
    assert_eq!(deferred.execution.substate, Substate::Attention);
    assert!(deferred.execution.reason.contains("Launch of bad failed"));
    // The budget frees while attention is set: the failure stays visible.
    settle_run(&service, &settings, &one.runs[0].run_id.clone());
    let freed = service.get(project, &mission.id).unwrap();
    assert_eq!(freed.runs.len(), 1);
    assert_eq!(freed.execution.substate, Substate::Attention);
    assert!(in_attention(&settings, &mission.id));
    let resumed = service.resume(project, &mission.id).unwrap();
    assert_eq!(resumed.runs.len(), 2);
    assert_eq!(resumed.runs[1].label, "two");
    assert_eq!(resumed.execution.substate, Substate::Running);
}

/// Seeds a run record the mission owns, with the pending result `create_run` writes.
fn owned_run(
    settings: &SparkSettings,
    project: &str,
    mission_id: &str,
    run_id: &str,
    status: &str,
) {
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    spark_storage::workspace_missions::MissionRepository::new(&root)
        .transact::<WorkspaceError>(mission_id, |value| {
            let mut value = value.unwrap();
            value["runs"].as_array_mut().unwrap().push(json!({"run_id":run_id,"label":"work","role":"work","launched_at":"t","launched_by_event":"e","status":"running"}));
            Ok(value)
        })
        .unwrap();
    let mut record = attractor_core::RunRecord::new(run_id, project);
    record.status = status.into();
    record.launch_context = Some(
        [(
            "context.spark_mission".to_string(),
            json!({"mission_id": mission_id, "label": "work", "role": "work"}),
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

#[test]
fn runs_ended_without_an_executor_deliver_exactly_one_terminal_event() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(
            project,
            mutation(
                json!({"fields":{"title":"Orphans","budget":{"concurrent_runs":2},"hooks":[
                    {"on":"mission.started","do":"ignore"},
                    {"on":"run.canceled","do":"ignore"},
                    {"on":"run.failed","do":"ignore"}
                ]}}),
            ),
        )
        .unwrap();
    service.start(project, &mission.id).unwrap();
    // Canceled with no executor attached: the cancel writes the result.
    owned_run(
        &settings,
        project,
        &mission.id,
        "run-canceled",
        "cancel_requested",
    );
    attractor_runtime::RuntimeControls::new(attractor_runtime::RunStore::for_settings(&settings))
        .mark_canceled("run-canceled", "no executor")
        .unwrap();
    for _ in 0..2 {
        service.deliver_run_events("run-canceled").unwrap();
    }
    // Failed before a restart while its result was still pending.
    owned_run(&settings, project, &mission.id, "run-failed", "failed");
    assert!(service.deliver_run_events("run-failed").unwrap().is_none());
    service.recover().unwrap();
    service.recover().unwrap();
    let kinds = kinds(&service, project, &mission.id);
    assert_eq!(kinds, ["mission.started", "run.canceled", "run.failed"]);
    let mission = service.get(project, &mission.id).unwrap();
    assert_eq!(
        mission
            .runs
            .iter()
            .map(|run| run.status.as_str())
            .collect::<Vec<_>>(),
        ["canceled", "failed"]
    );
    assert_eq!(mission.execution.substate, Substate::Idle);
}

#[test]
fn run_state_becomes_exactly_one_event_per_signal_and_terminal_status() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    write_flow(
        &settings,
        "work/signal.yaml",
        &tool_flow(
            "printf '%s' '{\"s\":{\"progress\":50}}'",
            json!({"context.mission.signal": "s"}),
            json!({}),
        ),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Signals","hooks":[
            {"on":"mission.started","do":{"launch":{"flow_name":"work/signal.yaml","label":"sig"}}},
            {"on":"run.signal","label":"sig","do":"ignore"},
            {"on":"run.completed","label":"sig","status":"completed","do":{"close":{"status":"done","reason":"signaled"}}}
        ]}})))
        .unwrap();
    let started = service.start(project, &mission.id).unwrap();
    let run_id = started.runs[0].run_id.clone();
    wait_terminal(&settings, &run_id);
    for _ in 0..3 {
        service.deliver_run_events(&run_id).unwrap();
    }
    let events = service.events(project, &mission.id, 0).unwrap();
    let signals: Vec<_> = events.iter().filter(|e| e.kind == "run.signal").collect();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].payload, json!({"progress": 50}));
    assert_eq!(signals[0].source, run_id);
    assert_eq!(
        events.iter().filter(|e| e.kind == "run.completed").count(),
        1
    );
    let closed = service.get(project, &mission.id).unwrap();
    assert_eq!(closed.fields.stage, Stage::Review);
    assert_eq!(closed.runs[0].status, "completed");
    // Unrelated runs deliver nothing.
    assert!(service.deliver_run_events("run-unknown").unwrap().is_none());
}

#[test]
fn terminal_events_wait_for_results_and_recovery_posts_missing_events() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Recover","hooks":[
                {"on":"mission.started","do":"ignore"},
                {"on":"run.completed","do":{"close":{"status":"done","reason":"recovered"}}}
            ]}})),
        )
        .unwrap();
    service.start(project, &mission.id).unwrap();
    // Simulate a run the mission launched before a restart.
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    let repo = spark_storage::workspace_missions::MissionRepository::new(&root);
    repo.transact::<WorkspaceError>(&mission.id, |value| {
        let mut value = value.unwrap();
        value["runs"] = json!([{"run_id":"run-owned","label":"work","role":"work","launched_at":"t","launched_by_event":"e","status":"running"}]);
        Ok(value)
    })
    .unwrap();
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let mut record = attractor_core::RunRecord::new("run-owned", project);
    record.status = "completed".into();
    record.launch_context = Some(
        [(
            "context.spark_mission".to_string(),
            json!({"mission_id": mission.id, "label": "work", "role": "work"}),
        )]
        .into(),
    );
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(
            &paths,
            &attractor_core::RunResult::pending("run-owned", "completed"),
        )
        .unwrap();
    assert!(service.deliver_run_events("run-owned").unwrap().is_none());
    assert!(!kinds(&service, project, &mission.id).contains(&"run.completed".to_string()));
    let mut result = attractor_core::RunResult::pending("run-owned", "completed");
    result.state = "ready".into();
    store.write_result(&paths, &result).unwrap();
    let recovered = service.recover().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        kinds(&service, project, &mission.id),
        ["mission.started", "run.completed"]
    );
    let mission = service.get(project, &mission.id).unwrap();
    assert_eq!(mission.runs[0].status, "completed");
    assert_eq!(mission.fields.stage, Stage::Review);
    assert!(service.recover().unwrap().is_empty());
}

#[test]
fn mission_reaction_coordinates_labeled_runs_hooks_and_batched_failure_to_review() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    write_flow(&settings, "work/ok.yaml", simple_flow());
    write_flow(
        &settings,
        "work/fail.yaml",
        &tool_flow("exit 1", json!({}), json!({})),
    );
    let launch = json!({"d":{"actions":[
        {"launch":{"flow_name":"work/ok.yaml","label":"first"}},
        {"launch":{"flow_name":"work/fail.yaml","label":"second"}},
        {"set_state":{"markdown":"Launched first and second"}}]}});
    let note = json!({"d":{"actions":[{"set_state":{"markdown":"Noted the message"}}]}});
    let finish = json!({"d":{"actions":[
        {"set_state":{"markdown":"Second failed after follow-up completed"}},
        {"close":{"status":"done","reason":"Handled both outcomes"}}]}});
    let script = format!(
        "case \"$MISSION\" in\n  *'\"kind\":\"run.failed\"'*) printf '%s' '{finish}' ;;\n  *'\"kind\":\"human.message\"'*) printf '%s' '{note}' ;;\n  *) printf '%s' '{launch}' ;;\nesac"
    );
    write_flow(
        &settings,
        "missions/react.yaml",
        &tool_flow(
            &script,
            json!({"context.mission.directive": "d"}),
            json!({"MISSION": "context.mission"}),
        ),
    );
    let service = WorkspaceMissionService::new(settings.clone());
    let mission = service
        .create(project, mutation(json!({"fields":{"title":"Ship it","description":"Deliver the change","hooks":[
            {"on":"run.completed","label":"first","do":{"launch":{"flow_name":"work/ok.yaml","label":"follow-up"}}}
        ]}})))
        .unwrap();
    let id = mission.id.clone();
    let mission = service.start(project, &id).unwrap();
    settle_run(&service, &settings, run_of(&mission, "reaction"));
    let mission = service.get(project, &id).unwrap();
    assert_eq!(mission.state, "Launched first and second");
    assert_eq!(mission.execution.substate, Substate::Running);
    settle_run(&service, &settings, run_of(&mission, "first"));
    let mission = service.get(project, &id).unwrap();
    // The hook, not a reaction, launched the follow-up.
    assert_eq!(
        mission
            .runs
            .iter()
            .filter(|r| r.role == Role::Reaction)
            .count(),
        1
    );
    // A human message starts a reaction; the remaining outcomes queue behind it.
    let mission = service
        .post_event(project, &id, post("human.message", None))
        .unwrap();
    assert_eq!(mission.execution.substate, Substate::Reasoning);
    let noting = run_of(&mission, "reaction").to_string();
    settle_run(&service, &settings, run_of(&mission, "second"));
    settle_run(&service, &settings, run_of(&mission, "follow-up"));
    let mission = service.get(project, &id).unwrap();
    assert_eq!(mission.pending_events.len(), 2);
    settle_run(&service, &settings, &noting);
    let mission = service.get(project, &id).unwrap();
    let last = run_of(&mission, "reaction").to_string();
    assert_ne!(last, noting);
    let batch = &launch_context(&settings, &last)["context.mission"]["events"];
    let batch_kinds: Vec<_> = batch
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(batch_kinds, ["run.failed", "run.completed"]);
    settle_run(&service, &settings, &last);
    let mission = service.get(project, &id).unwrap();
    assert_eq!(mission.fields.stage, Stage::Review);
    assert_eq!(mission.state, "Second failed after follow-up completed");
    assert_eq!(
        mission.closed.as_ref().unwrap().reason,
        "Handled both outcomes"
    );
    assert_eq!(mission.execution.substate, Substate::Idle);
    // Roster, inbox, and activity agree.
    let roster: Vec<_> = mission
        .runs
        .iter()
        .map(|r| (r.label.as_str(), r.status.as_str()))
        .collect();
    assert_eq!(
        roster,
        [
            ("reaction", "completed"),
            ("first", "completed"),
            ("second", "failed"),
            ("follow-up", "completed"),
            ("reaction", "completed"),
            ("reaction", "completed"),
        ]
    );
    let events = service.events(project, &id, 0).unwrap();
    for run in &mission.runs {
        let terminal = events
            .iter()
            .filter(|e| e.source == run.run_id && e.kind.starts_with("run."))
            .count();
        assert_eq!(terminal, 1, "{}", run.label);
    }
    assert_eq!(mission.cursor, events.last().unwrap().seq);
    let notes: Vec<_> = mission
        .activity
        .iter()
        .map(|a| a["note"].as_str().unwrap())
        .collect();
    assert_eq!(
        notes,
        [
            "",
            "Started mission",
            "Closed as done: Handled both outcomes"
        ]
    );
}

#[test]
fn ordinary_run_request_requires_approval_and_launches_without_mission_integration() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.yaml", simple_flow());
    seed_conversation(&settings, project, "conversation-task");
    let missions = WorkspaceMissionService::new(settings.clone());
    let mission = missions
        .create(
            project,
            mutation(json!({"fields":{"title":"Approved work","stage":"ready"}})),
        )
        .unwrap();
    let conversations = WorkspaceConversationService::new(settings.clone());
    let created = conversations
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.yaml".into(),
                summary: "Execute approved work".into(),
                execution_profile_id: Some("native".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let snapshot = conversations
        .review_flow_run_request(
            "conversation-task",
            &created.flow_run_request_id,
            FlowRunRequestReviewRequest {
                project_path: project.into(),
                disposition: "approved".into(),
                message: "Approved".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let artifact = &snapshot["flow_run_requests"][0];
    assert_eq!(artifact["status"], "launched", "{artifact}");
    let run_id = artifact["run_id"].as_str().unwrap();
    wait_terminal(&settings, run_id);
    assert!(missions.deliver_run_events(run_id).unwrap().is_none());
    let unchanged = missions.get(project, &mission.id).unwrap();
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.fields.stage, Stage::Ready);
}

#[test]
fn mission_listing_orders_creation_then_id_without_consulting_run_storage() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = project(&settings);
    let project = project.as_str();
    let service = WorkspaceMissionService::new(settings.clone());
    let mut mission = service
        .create(project, mutation(json!({"fields":{"title":"Seed"}})))
        .unwrap();
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    let repo = spark_storage::workspace_missions::MissionRepository::new(&root);
    for (id, at) in [
        ("mission-z", "2026-01-01"),
        ("mission-b", "2026-01-02"),
        ("mission-a", "2026-01-02"),
    ] {
        mission.id = id.into();
        mission.created_at = at.into();
        repo.transact::<WorkspaceError>(id, |_| Ok(serde_json::to_value(&mission).unwrap()))
            .unwrap();
    }
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record: attractor_core::RunRecord::new("run-unreadable", project),
            ..Default::default()
        })
        .unwrap();
    // Corrupt run metadata must have no bearing on mission reads.
    std::fs::write(paths.run_json(), "invalid json").unwrap();
    let ids: Vec<_> = service
        .list(project)
        .unwrap()
        .into_iter()
        .map(|mission| mission.id)
        .collect();
    assert_eq!(&ids[..3], &["mission-z", "mission-a", "mission-b"]);
}
