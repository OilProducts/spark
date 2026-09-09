use super::review_artifact_contracts::{
    seed_conversation, settings, simple_flow, write_flow, write_native_execution_profile,
};
use serde_json::{json, Value};
use spark_workspace::{
    tasks::{Stage, TaskMutation, TaskReference, WorkspaceTaskService},
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
    let mut task = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Manual delivery"}})),
        )
        .unwrap();
    assert_eq!(task.fields.stage, Stage::Backlog);
    for stage in ["planning", "ready", "in_progress", "review"] {
        task = service.update(project, &task.id, mutation(json!({"revision":task.revision,"fields":{"stage":stage,"blocked":"Review constraint","needs_input":"Which version?"}}))).unwrap();
    }
    let revision = task.revision;
    assert!(service
        .update(
            project,
            &task.id,
            mutation(json!({"revision":revision,"fields":{"stage":"done"}}))
        )
        .is_err());
    assert_eq!(service.get(project, &task.id).unwrap().revision, revision);
    task = service.update(project, &task.id, mutation(json!({"revision":revision,"fields":{"stage":"done"},"note":"Manually reviewed acceptance criteria"}))).unwrap();
    assert!(task.fields.runs.is_empty());
    task = service.update(project, &task.id, mutation(json!({"revision":task.revision,"fields":{"stage":"planning","blocked":"","needs_input":"","archived":true}}))).unwrap();
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
fn task_links_are_validated_project_scoped_and_many_to_many() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    std::fs::write(settings.project_root.join("decision.md"), "Decision").unwrap();
    seed_conversation(&settings, project, "conversation-task");
    seed_conversation(&settings, project, "conversation-task-two");
    let service = WorkspaceTaskService::new(settings.clone());
    let first = service.create(project, mutation(json!({"fields":{"title":"One","conversations":["conversation-task","conversation-task-two"],"artifacts":["decision.md"]},"actor":"assistant","conversation_id":"conversation-task"}))).unwrap();
    let second = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Two","conversations":["conversation-task","conversation-task-two"]}})),
        )
        .unwrap();
    assert_eq!(first.fields.conversations, second.fields.conversations);
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let other = other.to_str().unwrap();
    assert!(service.get(other, &first.id).is_err());
    assert!(service.get(project, "../outside").is_err());
    assert!(service.list(other).unwrap().is_empty());
    assert!(service
        .update(
            other,
            &first.id,
            mutation(json!({"revision":1,"note":"Wrong project"}))
        )
        .is_err());
    for fields in [
        json!({"runs":[{"run_id":"unknown"}]}),
        json!({"conversations":["unknown"]}),
        json!({"artifacts":["../outside"]}),
        json!({"artifacts":["absent.md"]}),
        json!({"stage":"blocked"}),
        json!({"priority":4}),
        json!({"unknown":true}),
    ] {
        assert!(service
            .update(
                project,
                &first.id,
                mutation(json!({"revision":1,"fields":fields}))
            )
            .is_err());
    }
    assert!(service
        .create(
            other,
            mutation(
                json!({"fields":{"title":"Wrong scope","conversations":["conversation-task","conversation-task-two"]}})
            )
        )
        .is_err());
    assert_eq!(service.get(project, &first.id).unwrap().revision, 1);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            temp.path().join("outside.md"),
            settings.project_root.join("escape.md"),
        )
        .unwrap();
        std::fs::write(temp.path().join("outside.md"), "outside").unwrap();
        assert!(service
            .update(
                project,
                &first.id,
                mutation(json!({"revision":1,"fields":{"artifacts":["escape.md"]}}))
            )
            .is_err());
    }
    let store = attractor_runtime::RunStore::for_settings(&settings);
    for (id, status) in [("run-success", "completed"), ("run-failed", "failed")] {
        let mut record = attractor_core::RunRecord::new(id, project);
        record.status = status.into();
        store
            .create_run(attractor_runtime::CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap();
    }
    let linked = service.update(project, &first.id, mutation(json!({"revision":1,"fields":{"stage":"ready","runs":[{"run_id":"run-success","stage":"planning"},{"run_id":"run-failed","stage":"planning"}]}}))).unwrap();
    assert_eq!(linked.fields.runs.len(), 2);
    assert_eq!(
        service.get(project, &first.id).unwrap().fields.stage,
        Stage::Ready
    );
    assert!(service
        .create(
            other,
            mutation(
                json!({"fields":{"title":"Wrong run scope","runs":[{"run_id":"run-success"}]}})
            )
        )
        .is_err());
}

#[test]
fn task_run_request_survives_approval_launch_and_recovery_without_duplicate_links() {
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
    assert!(conversations
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                task: Some(TaskReference {
                    task_id: "task-missing".into(),
                    stage: None
                }),
                flow_name: "ops/review.yaml".into(),
                summary: "Invalid association".into(),
                ..Default::default()
            }
        )
        .is_err());
    let created = conversations
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                task: Some(TaskReference {
                    task_id: task.id.clone(),
                    stage: Some(Stage::InProgress),
                }),
                flow_name: "ops/review.yaml".into(),
                summary: "Execute approved work".into(),
                execution_profile_id: Some("native".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(tasks.get(project, &task.id).unwrap().fields.runs.is_empty());
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
    assert_eq!(artifact["task"]["task_id"], task.id);
    let linked = tasks.get(project, &task.id).unwrap();
    assert_eq!(linked.fields.runs.len(), 1);
    assert_eq!(
        linked.fields.runs[0].run_id,
        artifact["run_id"].as_str().unwrap()
    );
    assert_eq!(linked.fields.runs[0].stage, Some(Stage::InProgress));
    assert_eq!(linked.fields.stage, Stage::Ready);
    assert_eq!(linked.fields.conversations, vec!["conversation-task"]);
    // Simulate loss of the task-side association after durable run creation.
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    spark_storage::workspace_tasks::TaskRepository::new(&root)
        .transact::<WorkspaceError>(&task.id, |_| Ok(serde_json::to_value(&task).unwrap()))
        .unwrap();
    let restarted = WorkspaceTaskService::new(settings.clone());
    let recovered = restarted.get(project, &task.id).unwrap();
    assert_eq!(recovered.fields.runs.len(), 1);
    assert_eq!(recovered.activity.len(), 2);
    assert_eq!(
        restarted.get(project, &task.id).unwrap().revision,
        recovered.revision
    );
    let unlinked = restarted
        .update(
            project,
            &task.id,
            mutation(json!({"revision":recovered.revision,"fields":{"runs":[]}})),
        )
        .unwrap();
    assert!(restarted
        .get(project, &task.id)
        .unwrap()
        .fields
        .runs
        .is_empty());
    assert_eq!(unlinked.revision, recovered.revision + 1);
}

#[test]
fn task_attention_uses_descendant_questions_and_existing_answers() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.project_root).unwrap();
    let project = settings.project_root.to_str().unwrap();
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let parent = attractor_core::RunRecord::new("run-parent", project);
    store
        .create_run(attractor_runtime::CreateRunRequest {
            record: parent,
            ..Default::default()
        })
        .unwrap();
    let mut child = attractor_core::RunRecord::new("run-child", project);
    child.parent_run_id = Some("run-parent".into());
    child.status = "waiting".into();
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record: child,
            ..Default::default()
        })
        .unwrap();
    let mut question = attractor_core::RawRuntimeEvent::new("human_gate", "run-child");
    question
        .payload
        .insert("question_id".into(), json!("question-task"));
    question
        .payload
        .insert("prompt".into(), json!("Approve evidence?"));
    store.append_event(&paths, question).unwrap();
    let service = WorkspaceTaskService::new(settings.clone());
    let task = service
        .create(
            project,
            mutation(json!({"fields":{"title":"Needs review","runs":[{"run_id":"run-parent"}]}})),
        )
        .unwrap();
    let board = service.board(project).unwrap();
    assert_eq!(
        board["attention"],
        json!([{"task_id":task.id,"run_id":"run-child"}])
    );
    let mut answer = attractor_core::RawRuntimeEvent::new("InterviewCompleted", "run-child");
    answer
        .payload
        .insert("question_id".into(), json!("question-task"));
    answer.payload.insert("answer".into(), json!("Approved"));
    store.append_event(&paths, answer).unwrap();
    assert_eq!(service.board(project).unwrap()["attention"], json!([]));
    assert_eq!(service.get(project, &task.id).unwrap().revision, 1);
}
