use super::review_artifact_contracts::{
    seed_conversation, settings, simple_flow, write_flow, write_native_execution_profile,
};
use serde_json::{json, Value};
use spark_workspace::{
    tasks::{Stage, TaskMutation, WorkspaceTaskService},
    FlowRunRequestCreateByHandleRequest, FlowRunRequestReviewRequest, WorkspaceConversationService,
    WorkspaceError,
};
fn mutation(value: Value) -> TaskMutation {
    serde_json::from_value(value).unwrap()
}

#[test]
fn tasks_persist_atomically_reject_stale_writers_and_support_manual_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    let service = WorkspaceTaskService::new(settings.clone());
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    assert_eq!(registry.read_project_record(project).unwrap(), None);
    assert!(!registry.projects_root().exists());
    let mut task = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Manual delivery"}})),
        )
        .unwrap();
    assert_eq!(task.fields.stage, Stage::Backlog);
    assert!(registry.read_project_record(project).unwrap().is_some());
    for stage in ["planning", "ready", "in_progress", "review"] {
        task = service
            .update(
                project,
                &task.id,
                mutation(json!({"revision":task.revision,"fields":{"stage":stage}})),
            )
            .unwrap();
    }
    let revision = task.revision;
    task = service
        .update(
            project,
            &task.id,
            mutation(json!({"revision":revision,"fields":{"stage":"done"}})),
        )
        .unwrap();
    task = service
        .update(
            project,
            &task.id,
            mutation(
                json!({"revision":task.revision,"fields":{"stage":"planning","archived":true}}),
            ),
        )
        .unwrap();
    assert_eq!(task.fields.stage, Stage::Planning);
    assert!(task.fields.archived);
    assert_eq!(task.activity.len() as u64, task.revision);
    let restarted = WorkspaceTaskService::new(settings.clone());
    assert_eq!(
        serde_json::to_value(&task).unwrap(),
        serde_json::to_value(restarted.get(project, &task.id).unwrap()).unwrap()
    );
    let before = serde_json::to_value(&task).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let outcomes = std::thread::scope(|scope| {
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|title| {
                let barrier = barrier.clone();
                let settings = settings.clone();
                let task = task.clone();
                scope.spawn(move || {
                    barrier.wait();
                    WorkspaceTaskService::new(settings).update(
                        project,
                        &task.id,
                        mutation(json!({"revision":task.revision,"fields":{"title":title}})),
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
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, Err(WorkspaceError::Conflict(_))))
            .count(),
        1
    );
    let final_task = restarted.get(project, &task.id).unwrap();
    assert_eq!(final_task.revision, task.revision + 1);
    assert_eq!(
        final_task.activity.last().unwrap()["before"],
        before["fields"]
    );
    assert_eq!(
        final_task.activity.last().unwrap()["after"],
        serde_json::to_value(&final_task.fields).unwrap()
    );
}

#[test]
fn tasks_reject_removed_fields_and_preserve_scope_and_attribution() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    let service = WorkspaceTaskService::new(settings.clone());
    let task = service.create(project, mutation(json!({"fields":{"title":"One","stage":"done"},"actor":"assistant","note":"Useful detail"}))).unwrap();
    assert_eq!(task.fields.stage, Stage::Done);
    assert_eq!(task.activity[0]["actor"], "assistant");
    assert_eq!(task.activity[0]["note"], "Useful detail");
    assert!(task.activity[0].get("conversation_id").is_none());
    assert_eq!(
        serde_json::to_value(&task.fields).unwrap(),
        json!({"title":"One","description":"","stage":"done","archived":false})
    );
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let other = other.to_str().unwrap();
    assert!(service.get(other, &task.id).is_err());
    assert!(service.get(project, "../outside").is_err());
    assert!(service.list(other).unwrap().is_empty());
    assert!(service
        .update(
            other,
            &task.id,
            mutation(json!({"revision":1,"fields":{"title":"Wrong"}}))
        )
        .is_err());
    for key in [
        "priority",
        "acceptance_criteria",
        "next_action",
        "blocked",
        "needs_input",
        "conversations",
        "artifacts",
        "runs",
    ] {
        assert!(
            service
                .update(
                    project,
                    &task.id,
                    mutation(json!({"revision":1,"fields":{key:null}}))
                )
                .is_err(),
            "{key}"
        );
    }
    for fields in [json!({"title":"  "}), json!({"stage":"unknown"})] {
        assert!(service
            .create(project, mutation(json!({"fields":fields})))
            .is_err());
    }
    assert!(serde_json::from_value::<TaskMutation>(
        json!({"fields":{"title":"No provenance"},"conversation_id":"old"})
    )
    .is_err());
    let second = service
        .create(project, mutation(json!({"fields":{"title":"Two"}})))
        .unwrap();
    assert_eq!(
        service.board(project).unwrap(),
        json!({"tasks":[task, second]})
    );
}

#[test]
fn ordinary_run_request_requires_approval_and_launches_without_task_integration() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.yaml", simple_flow());
    seed_conversation(&settings, project, "conversation-task");
    let tasks = WorkspaceTaskService::new(settings.clone());
    let task = tasks
        .create(
            project,
            mutation(json!({"fields":{"title":"Approved work","stage":"ready"}})),
        )
        .unwrap();
    let conversations = WorkspaceConversationService::new(settings.clone());
    assert!(
        serde_json::from_value::<FlowRunRequestCreateByHandleRequest>(
            json!({"task":{"task_id":"removed"},"flow_name":"ops/review.yaml","summary":"Invalid"})
        )
        .is_err()
    );
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
    assert_eq!(tasks.get(project, &task.id).unwrap().revision, 1);
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
    assert!(artifact.get("task").is_none());
    assert!(artifact["run_id"].as_str().is_some());
    assert_eq!(tasks.get(project, &task.id).unwrap().revision, 1);
    assert_eq!(
        tasks.get(project, &task.id).unwrap().fields.stage,
        Stage::Ready
    );
}

#[test]
fn task_listing_orders_creation_then_id_without_consulting_run_storage() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    let service = WorkspaceTaskService::new(settings.clone());
    let mut task = service
        .create(project, mutation(json!({"fields":{"title":"Seed"}})))
        .unwrap();
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    let repo = spark_storage::workspace_tasks::TaskRepository::new(&root);
    for (id, at) in [
        ("task-z", "2026-01-01"),
        ("task-b", "2026-01-02"),
        ("task-a", "2026-01-02"),
    ] {
        task.id = id.into();
        task.created_at = at.into();
        repo.transact::<WorkspaceError>(id, |_| Ok(serde_json::to_value(&task).unwrap()))
            .unwrap();
    }
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record: attractor_core::RunRecord::new("run-unreadable", project),
            ..Default::default()
        })
        .unwrap();
    // Corrupt run metadata must have no bearing on task reads.
    std::fs::write(paths.run_json(), "invalid json").unwrap();
    assert!(store.read_run_meta("run-unreadable").is_err());
    let ids: Vec<_> = service
        .list(project)
        .unwrap()
        .into_iter()
        .map(|task| task.id)
        .collect();
    assert_eq!(&ids[..3], &["task-z", "task-a", "task-b"]);
}
