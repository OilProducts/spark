use super::mission_contracts::{project, tool_flow, wait_for, wait_terminal, GatedAgent};
use super::review_artifact_contracts::{settings, write_native_execution_profile};
use serde_json::{json, Value};
use spark_agent_adapter::{AgentTurnRequest, AGENT_INSTRUCTIONS_METADATA_KEY};
use spark_common::settings::SparkSettings;
use spark_storage::workspace_records::RecordRepository;
use spark_workspace::{
    inbox::{install_runtime, InboxPost, InboxRuntime},
    missions::WorkspaceMissionService,
    roles::{RoleRecord, RoleState, WorkspaceRoleService, CHIEF_OF_STAFF, COORDINATOR},
    ConversationSettingsUpdate, ConversationTurnRequest, FlowRunRequestCreateByHandleRequest,
    WorkspaceConversationService, WorkspaceError,
};
use std::sync::Arc;

const COORDINATOR_FILE: &str = include_str!("../../../spark-assets/assets/roles/coordinator.md");
const ASSISTANT_FILE: &str = include_str!("../../../spark-assets/assets/roles/assistant.md");
const CHIEF_OF_STAFF_FILE: &str =
    include_str!("../../../spark-assets/assets/roles/chief-of-staff.md");

/// Spark writes the bundled library on start; tests seed its roles.
pub(crate) fn seed_bundled_roles(settings: &SparkSettings) {
    let roles = settings.bundled_dir().join("roles");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(roles.join("coordinator.md"), COORDINATOR_FILE).unwrap();
    std::fs::write(roles.join("assistant.md"), ASSISTANT_FILE).unwrap();
    std::fs::write(roles.join("chief-of-staff.md"), CHIEF_OF_STAFF_FILE).unwrap();
}

struct Harness {
    _temp: tempfile::TempDir,
    settings: SparkSettings,
    project: String,
    agent: Arc<GatedAgent>,
    roles: WorkspaceRoleService,
}
impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let project = project(&settings);
        write_native_execution_profile(&settings);
        seed_bundled_roles(&settings);
        let agent = Arc::new(GatedAgent::default());
        install_runtime(
            &settings,
            InboxRuntime {
                agent_turn_backend: agent.clone(),
                publish: Arc::new(|_| {}),
            },
        );
        Self {
            roles: WorkspaceRoleService::new(settings.clone()),
            _temp: temp,
            settings,
            project,
            agent,
        }
    }
    fn get(&self, name: &str) -> RoleRecord {
        self.roles.get(&self.project, name).unwrap()
    }
    fn conversations(&self) -> WorkspaceConversationService {
        WorkspaceConversationService::new_with_agent_turn_backend(
            self.settings.clone(),
            self.agent.clone(),
        )
    }
    fn request(&self, index: usize) -> AgentTurnRequest {
        self.wait_turns(index + 1);
        self.agent.requests.lock().unwrap()[index].clone()
    }
    fn wait_turns(&self, count: usize) {
        wait_for(&format!("{count} turns"), || {
            self.agent.requests.lock().unwrap().len() >= count
        });
    }
    fn wait_idle(&self, name: &str) {
        wait_for("an idle role", || {
            self.get(name).state != RoleState::Working
        });
    }
    /// Lets the one running turn finish and waits for the role to settle.
    fn finish_turn(&self, name: &str) {
        self.agent.release(1);
        self.wait_idle(name);
    }
    fn handle(&self, name: &str) -> String {
        self.conversations()
            .get_snapshot(&self.get(name).conversation_id, Some(&self.project))
            .unwrap()["conversation_handle"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn message(&self, name: &str, text: &str) -> RoleRecord {
        self.roles
            .post(
                &self.project,
                name,
                InboxPost {
                    kind: "role.message".into(),
                    payload: json!({"from": "a test", "message": text}),
                    ..Default::default()
                },
            )
            .unwrap()
    }
    /// The role's `spark convo run-request` from inside its turn.
    fn launch(&self, name: &str, flow: &str, summary: &str) -> Result<String, WorkspaceError> {
        WorkspaceConversationService::new(self.settings.clone())
            .create_flow_run_request_by_handle(
                &self.handle(name),
                FlowRunRequestCreateByHandleRequest {
                    flow_name: flow.into(),
                    summary: summary.into(),
                    execution_profile_id: Some("native".into()),
                    ..Default::default()
                },
            )
            .map(|response| response["run_id"].as_str().unwrap().to_string())
    }
    fn edit_record(&self, name: &str, edit: impl FnOnce(&mut Value)) {
        let root = spark_storage::ProjectRegistry::new(&self.settings.data_dir)
            .ensure_project_paths(&self.project)
            .unwrap()
            .root;
        RecordRepository::roles(&root)
            .transact::<WorkspaceError>(name, |value| {
                let mut value = value.unwrap();
                edit(&mut value);
                Ok(value)
            })
            .unwrap();
    }
    fn org_role(&self, name: &str, text: &str) {
        let roles =
            spark_common::project::org_dir(std::path::Path::new(&self.project)).join("roles");
        std::fs::create_dir_all(&roles).unwrap();
        std::fs::write(roles.join(format!("{name}.md")), text).unwrap();
    }
    /// A user message typed into a conversation; its turn is left running.
    fn user_turn(&self, conversation_id: &str) -> AgentTurnRequest {
        let (prepared, _) = self
            .conversations()
            .send_turn(
                conversation_id,
                ConversationTurnRequest {
                    project_path: self.project.clone(),
                    message: "Hello".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        prepared.unwrap().agent_turn_request
    }
}

fn frame(request: &AgentTurnRequest) -> String {
    request.metadata[AGENT_INSTRUCTIONS_METADATA_KEY]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn missions_roles_and_ordinary_chats_each_get_their_own_frame() {
    let harness = Harness::new();
    let coordinator = harness.get(COORDINATOR).conversation_id;
    let role = frame(&harness.user_turn(&coordinator));
    assert!(role.contains("Org changes"), "{role}");
    for rule in [
        NEVER_CHANGE,
        "`software-development/design-change` when the approach needs deciding",
        "`software-development/implement-change` when it's clear",
        "check its design against what the user asked for",
        "tell the user exactly where it diverges and ask before implementing",
    ] {
        assert!(role.contains(rule), "{rule}");
    }
    assert!(
        role.contains("standing Coordinator (coordinator)"),
        "{role}"
    );
    assert!(role.contains("memory/coordinator"));
    assert!(role.contains("Only the Chief of Staff role changes files in the org repo"));
    let chat = frame(&harness.user_turn("chat-1"));
    assert!(
        chat.starts_with("You are the Spark workspace assistant."),
        "{chat}"
    );
    assert!(chat.contains("Org changes"), "{chat}");
    assert!(!chat.contains(NEVER_CHANGE), "{chat}");
    assert!(!chat.contains("standing Coordinator ("), "{chat}");
    assert!(chat.contains("memory/coordinator"));
    assert!(chat.contains("never edit it"));
    let missions = WorkspaceMissionService::new(harness.settings.clone());
    let mission = missions
        .create(
            &harness.project,
            serde_json::from_value(json!({"fields": {"title": "Ship"}})).unwrap(),
        )
        .unwrap();
    missions.start(&harness.project, &mission.id).unwrap();
    let mission_frame = frame(&harness.request(0));
    assert!(mission_frame.contains(&format!("Mission ID: {}", mission.id)));
    assert!(!mission_frame.contains("Org changes"));
    assert!(!mission_frame.contains(NEVER_CHANGE));
    harness.agent.release(1);
}

const NEVER_CHANGE: &str = "Never edit, create, delete or commit project files yourself";

#[test]
fn an_org_assistant_role_replaces_the_bundled_one_for_ordinary_chats() {
    let harness = Harness::new();
    harness.org_role(
        spark_workspace::roles::ASSISTANT,
        "---\ntype: Role\ntitle: Assistant\n---\n\nYou are this org's assistant.\n",
    );
    let chat = frame(&harness.user_turn("chat-1"));
    assert!(chat.starts_with("You are this org's assistant."), "{chat}");
    assert!(
        !chat.contains("You are the Spark workspace assistant."),
        "{chat}"
    );
}

#[test]
fn a_conversation_model_beats_its_role_file_which_beats_the_project() {
    let harness = Harness::new();
    let paths = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .ensure_project_paths(&harness.project)
        .unwrap();
    let text = std::fs::read_to_string(&paths.project_file).unwrap_or_default();
    std::fs::write(
        &paths.project_file,
        format!("{text}\n[model_settings]\nprovider = \"codex\"\nmodel = \"project-model\"\n"),
    )
    .unwrap();
    harness.org_role(
        COORDINATOR,
        &COORDINATOR_FILE.replacen("---\n", "---\nexecution:\n  llm_model: role-model\n", 1),
    );
    let conversations = harness.conversations();
    let models = |id: &str| {
        conversations
            .get_snapshot(id, Some(&harness.project))
            .unwrap()["settings"]["models"]
            .clone()
    };
    assert_eq!(models("chat-1")["effective"]["model"], "project-model");
    assert_eq!(models("chat-1")["source"], "project");
    let coordinator = harness.get(COORDINATOR).conversation_id;
    assert_eq!(models(&coordinator)["effective"]["model"], "role-model");
    assert_eq!(models(&coordinator)["source"], "role");
    // The role keeps the project's provider when it names only a model.
    assert_eq!(models(&coordinator)["effective"]["provider"], "codex");
    harness.message(COORDINATOR, "One");
    assert_eq!(harness.request(0).model.as_deref(), Some("role-model"));
    harness.finish_turn(COORDINATOR);
    let revision = conversations
        .get_snapshot(&coordinator, Some(&harness.project))
        .unwrap()["revision"]
        .to_string();
    conversations
        .update_conversation_settings(
            &coordinator,
            ConversationSettingsUpdate {
                project_path: harness.project.clone(),
                provider: Some("codex".into()),
                model: Some("conversation-model".into()),
                expected_revision: Some(revision),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        models(&coordinator)["effective"]["model"],
        "conversation-model"
    );
    assert_eq!(models(&coordinator)["source"], "conversation");
    harness.message(COORDINATOR, "Two");
    assert_eq!(
        harness.request(1).model.as_deref(),
        Some("conversation-model")
    );
    harness.finish_turn(COORDINATOR);
}

#[test]
fn the_coordinator_exists_unsaved_until_its_first_write() {
    let harness = Harness::new();
    let root = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .ensure_project_paths(&harness.project)
        .unwrap()
        .root;
    let listed = harness.roles.list(&harness.project).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, COORDINATOR);
    assert_eq!(listed[0].title, "Coordinator");
    harness
        .roles
        .process(&harness.project, COORDINATOR)
        .unwrap();
    assert!(!root.join("roles/coordinator.json").exists());
    // Its Threads row is a placeholder until the conversation exists.
    let threads = harness
        .conversations()
        .list_project_conversations(&harness.project)
        .unwrap();
    let pinned = &threads[0];
    assert_eq!(pinned.conversation_id, listed[0].conversation_id);
    assert_eq!(pinned.title, "Coordinator");
    assert_eq!(pinned.standing_role.as_ref().unwrap().name, COORDINATOR);
    // A role that was never started does not exist.
    assert!(matches!(
        harness.roles.get(&harness.project, CHIEF_OF_STAFF),
        Err(WorkspaceError::NotFound(_))
    ));
    harness.message(COORDINATOR, "Hello");
    assert!(root.join("roles/coordinator.json").exists());
    harness.request(0);
    harness.finish_turn(COORDINATOR);
    let threads = harness
        .conversations()
        .list_project_conversations(&harness.project)
        .unwrap();
    assert_eq!(
        threads
            .iter()
            .filter(|thread| thread.standing_role.is_some())
            .count(),
        1
    );
}

#[test]
fn a_run_the_role_launched_reports_back_as_a_turn() {
    let harness = Harness::new();
    crate::install_named_flow(&harness.settings, "work/ok", tool_flow("true"));
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    let begin = harness.request(0);
    assert_eq!(
        begin.prompt,
        "Begin your work as this project's standing role."
    );
    assert!(frame(&begin).contains("You are this project's Chief of Staff."));
    let run = harness
        .launch(CHIEF_OF_STAFF, "work/ok", "Build it")
        .unwrap();
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let record = store
        .read_run_record(&store.find_run_root(&run).unwrap().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        json!(record.launch_context)["context.spark_owner"],
        json!({"kind": "role", "id": CHIEF_OF_STAFF})
    );
    // Its run is in flight, so the role stays working after the turn ends.
    harness.agent.release(1);
    wait_terminal(&harness.settings, &run);
    spark_workspace::inbox::deliver_run_events(&harness.settings, &run).unwrap();
    assert_eq!(
        harness.request(1).prompt,
        format!("Run {run} (work/ok, \"Build it\") ended completed.")
    );
    harness.finish_turn(CHIEF_OF_STAFF);
    assert_eq!(
        harness.get(CHIEF_OF_STAFF).inbox.runs[0].status,
        "completed"
    );
}

#[test]
fn a_stopped_role_records_events_and_delivers_them_when_started_again() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF);
    let stopped = harness
        .roles
        .stop(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    assert_eq!(stopped.state, RoleState::Stopped);
    let recorded = harness.message(CHIEF_OF_STAFF, "While you were out");
    assert_eq!(recorded.inbox.event_seq, recorded.inbox.cursor + 1);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 1);
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    assert_eq!(
        harness.request(1).prompt,
        "Message from a test: While you were out"
    );
    harness.finish_turn(CHIEF_OF_STAFF);
}

#[test]
fn launches_over_either_budget_are_refused() {
    let harness = Harness::new();
    crate::install_named_flow(&harness.settings, "work/ok", tool_flow("true"));
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.edit_record(CHIEF_OF_STAFF, |role| {
        role["budget"]["concurrent_runs"] = json!(0)
    });
    let refused = harness
        .launch(CHIEF_OF_STAFF, "work/ok", "One")
        .unwrap_err();
    assert!(
        refused.to_string().contains("concurrent_runs (0)"),
        "{refused}"
    );
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    harness.edit_record(CHIEF_OF_STAFF, |role| {
        role["budget"]["concurrent_runs"] = json!(4);
        // Launches older than a day no longer count.
        role["recent_launches"] = json!([now - 90_000, now, now]);
        role["budget"]["runs_per_day"] = json!(2);
    });
    let refused = harness
        .launch(CHIEF_OF_STAFF, "work/ok", "Two")
        .unwrap_err();
    assert!(
        refused.to_string().contains("runs_per_day (2)"),
        "{refused}"
    );
    assert!(harness.get(CHIEF_OF_STAFF).inbox.runs.is_empty());
    harness.finish_turn(CHIEF_OF_STAFF);
}

#[test]
fn a_message_from_another_role_names_its_sender() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF);
    let handle = harness.handle(CHIEF_OF_STAFF);
    harness
        .roles
        .send(&harness.project, COORDINATOR, &handle, "Fixed the flow")
        .unwrap();
    let request = harness.request(1);
    assert_eq!(
        request.prompt,
        "Message from the chief-of-staff role: Fixed the flow"
    );
    assert_eq!(
        request.conversation_id,
        harness.get(COORDINATOR).conversation_id
    );
    harness.finish_turn(COORDINATOR);
    assert!(harness
        .roles
        .send(&harness.project, COORDINATOR, "no-such-handle", "Hi")
        .is_err());
}

#[test]
fn a_changed_persona_is_announced_once() {
    let harness = Harness::new();
    harness.message(COORDINATOR, "One");
    assert_eq!(harness.request(0).prompt, "Message from a test: One");
    harness.finish_turn(COORDINATOR);
    harness.org_role(
        COORDINATOR,
        "---\ntype: Role\ntitle: Coordinator\n---\n\nYou are a terse coordinator.\n",
    );
    harness.message(COORDINATOR, "Two");
    let changed = harness.request(1);
    assert!(
        changed
            .prompt
            .starts_with("Your instructions changed since your last turn"),
        "{}",
        changed.prompt
    );
    assert!(changed.prompt.ends_with("Message from a test: Two"));
    assert!(frame(&changed).starts_with("You are a terse coordinator."));
    harness.finish_turn(COORDINATOR);
    harness.message(COORDINATOR, "Three");
    assert_eq!(harness.request(2).prompt, "Message from a test: Three");
    harness.finish_turn(COORDINATOR);
}

#[test]
fn recovery_delivers_a_role_runs_missing_terminal_event() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF);
    // A run failed before a restart while its result was still pending.
    harness.edit_record(CHIEF_OF_STAFF, |role| {
        role["runs"] = json!([{"run_id": "run-lost", "flow_name": "work/seeded", "summary": "Seeded work", "launched_at": "t", "status": "running"}]);
    });
    let mut record = attractor_core::RunRecord::new("run-lost", &harness.project);
    record.status = "failed".into();
    record.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            json!({"kind": "role", "id": CHIEF_OF_STAFF}),
        )]
        .into(),
    );
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(
            &paths,
            &attractor_core::RunResult::pending("run-lost", "failed"),
        )
        .unwrap();
    assert!(harness
        .roles
        .deliver_run_events("run-lost")
        .unwrap()
        .is_none());
    harness.roles.recover().unwrap();
    assert_eq!(
        harness.request(1).prompt,
        "Run run-lost (work/seeded, \"Seeded work\") ended failed."
    );
    harness.finish_turn(CHIEF_OF_STAFF);
    assert_eq!(harness.get(CHIEF_OF_STAFF).inbox.runs[0].status, "failed");
    harness.roles.recover().unwrap();
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 2);
}

#[test]
fn a_question_from_a_run_the_role_launched_arrives_as_a_turn_with_answer_instructions() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF);
    harness.edit_record(CHIEF_OF_STAFF, |role| {
        role["runs"] = json!([{"run_id": "run-asks", "flow_name": "work/seeded", "summary": "Seeded work", "launched_at": "t", "status": "running"}]);
    });
    let mut record = attractor_core::RunRecord::new("run-asks", &harness.project);
    record.status = "running".into();
    record.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            json!({"kind": "role", "id": CHIEF_OF_STAFF}),
        )]
        .into(),
    );
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .append_event(
            &paths,
            attractor_runtime::human_gate_pending_event(
                "run-asks",
                "q-1",
                "clarification",
                "work",
                "Which branch?",
                None,
                vec![],
            ),
        )
        .unwrap();
    spark_workspace::inbox::deliver_run_events(&harness.settings, "run-asks").unwrap();
    let turn = harness.request(1);
    assert!(turn.prompt.contains("Which branch?"), "{}", turn.prompt);
    assert!(turn.prompt.contains("q-1"), "{}", turn.prompt);
    let frame = frame(&turn);
    assert!(
        frame.contains("spark run answer --run <owning run id>"),
        "{frame}"
    );
    assert!(frame.contains("Never guess an answer"), "{frame}");
    assert!(frame.contains("`spark run retry|continue|events|questions|answer ...`"));
    harness.agent.release(1);
}

#[test]
fn a_finishing_run_trims_the_roster_to_fifty_finished_and_keeps_in_flight_runs() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF);
    harness.edit_record(CHIEF_OF_STAFF, |role| {
        let entry = |id: String, status: &str| json!({"run_id": id, "flow_name": "work/seeded", "summary": "Seeded work", "launched_at": "t", "status": status});
        let mut runs: Vec<Value> = (0..50)
            .map(|index| entry(format!("run-done-{index}"), "completed"))
            .collect();
        runs.insert(10, entry("run-other".into(), "running"));
        runs.push(entry("run-lost".into(), "running"));
        role["runs"] = json!(runs);
    });
    let mut record = attractor_core::RunRecord::new("run-lost", &harness.project);
    record.status = "failed".into();
    record.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            json!({"kind": "role", "id": CHIEF_OF_STAFF}),
        )]
        .into(),
    );
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(
            &paths,
            &attractor_core::RunResult::pending("run-lost", "failed"),
        )
        .unwrap();
    // Recovery reconciles the run to its terminal status, which trims.
    harness.roles.recover().unwrap();
    harness.request(1);
    let runs = harness.get(CHIEF_OF_STAFF).inbox.runs;
    let ids: Vec<&str> = runs.iter().map(|run| run.run_id.as_str()).collect();
    assert_eq!(runs.iter().filter(|run| !run.in_flight()).count(), 50);
    assert!(!ids.contains(&"run-done-0"), "{ids:?}");
    assert!(ids.contains(&"run-done-1"), "{ids:?}");
    assert!(runs
        .iter()
        .any(|run| run.run_id == "run-other" && run.in_flight()));
    assert!(runs
        .iter()
        .any(|run| run.run_id == "run-lost" && run.status == "failed"));
    // run-other stays in flight, so the role keeps working after the turn.
    harness.agent.release(1);
}

#[test]
fn a_role_named_with_an_underscore_starts_talks_and_resumes() {
    let harness = Harness::new();
    let name = "release_manager";
    harness.org_role(name, CHIEF_OF_STAFF_FILE);
    let started = harness.roles.start(&harness.project, name).unwrap();
    assert_eq!(
        harness
            .roles
            .conversation_role(&harness.project, &started.conversation_id)
            .unwrap()
            .name,
        name
    );
    harness.request(0);
    harness.finish_turn(name);
    harness.roles.stop(&harness.project, name).unwrap();
    harness.message(name, "Cut the release");
    harness.roles.start(&harness.project, name).unwrap();
    assert_eq!(
        harness.request(1).prompt,
        "Message from a test: Cut the release"
    );
    harness.finish_turn(name);
    for unsafe_name in ["../outside", ".hidden", "a/b"] {
        assert!(harness.roles.start(&harness.project, unsafe_name).is_err());
        assert!(harness.roles.get(&harness.project, unsafe_name).is_err());
    }
}
