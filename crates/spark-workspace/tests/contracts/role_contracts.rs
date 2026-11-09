use super::mission_contracts::{project, tool_flow, wait_for, wait_terminal, GatedAgent};
use super::review_artifact_contracts::{
    seed_conversation, settings, write_native_execution_profile,
};
use serde_json::json;
use spark_agent_adapter::{AgentTurnRequest, AGENT_INSTRUCTIONS_METADATA_KEY};
use spark_common::settings::SparkSettings;
use spark_workspace::{
    inbox::{install_runtime, InboxRuntime, ThreadMessageRequest},
    roles::{RoleRecord, RoleState, WorkspaceRoleService},
    ConversationSettingsUpdate, ConversationTurnRequest, FlowRunRequestCreateByHandleRequest,
    WorkspaceConversationService, WorkspaceError,
};
use std::sync::Arc;

const COORDINATOR_FILE: &str = include_str!("../../../spark-assets/assets/roles/coordinator.md");
const ASSISTANT_FILE: &str = include_str!("../../../spark-assets/assets/roles/assistant.md");
const CHIEF_OF_STAFF_FILE: &str =
    include_str!("../../../spark-assets/assets/roles/chief-of-staff.md");

/// Bundled standing roles the tests start like any other.
const COORDINATOR_ROLE: &str = "coordinator";
const CHIEF_OF_STAFF_ROLE: &str = "chief-of-staff";

/// The handle of the chat that `Harness::message` sends from.
const SENDER: &str = "amber-anchor";

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
        let settings = settings(&temp.path().canonicalize().unwrap());
        let project = project(&settings);
        write_native_execution_profile(&settings);
        seed_bundled_roles(&settings);
        seed_conversation(&settings, &project, "chat-sender");
        let agent = Arc::new(GatedAgent::default());
        install_runtime(
            &settings,
            InboxRuntime {
                agent_turn_backend: agent.clone(),
                publish: Arc::new(|_| {}),
            },
        );
        Self {
            roles: crate::roles(settings.clone()),
            _temp: temp,
            settings,
            project,
            agent,
        }
    }
    fn get(&self, name: &str) -> RoleRecord {
        self.roles.get(&self.project, name).unwrap()
    }
    /// Starts a role and lets its first turn finish.
    fn start(&self, name: &str) {
        let before = self.agent.requests.lock().unwrap().len();
        self.roles.start(&self.project, name).unwrap();
        self.request(before);
        self.finish_turn(name);
    }
    fn conversations(&self) -> WorkspaceConversationService {
        crate::conversations_with_backend(self.settings.clone(), self.agent.clone())
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
    /// A `spark send` to the started role from the sender chat.
    fn message(&self, name: &str, text: &str) -> RoleRecord {
        spark_workspace::inbox::send_message(
            &self.settings,
            &crate::shared_store(&self.settings.data_dir),
            &self.handle(name),
            ThreadMessageRequest {
                conversation_handle: SENDER.into(),
                message: text.into(),
            },
        )
        .unwrap();
        self.get(name)
    }
    /// How a `message` from the sender chat reads in a batch.
    fn sent(&self, text: &str) -> String {
        format!(
            "Message from conversation {SENDER}, project {}:\n{text}",
            self.project
        )
    }
    /// The role's `spark convo run-request` from inside its turn.
    fn launch(&self, name: &str, flow: &str, summary: &str) -> Result<String, WorkspaceError> {
        crate::conversations(self.settings.clone())
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
    fn org_role(&self, name: &str, text: &str) {
        let roles = spark_common::project::definitions_dir(std::path::Path::new(&self.project))
            .join("roles");
        std::fs::create_dir_all(&roles).unwrap();
        std::fs::write(roles.join(format!("{name}.md")), text).unwrap();
    }
    /// A user message typed into a conversation; its turn is left running.
    fn user_turn(&self, conversation_id: &str) -> AgentTurnRequest {
        let conversations = self.conversations();
        let (prepared, _) = conversations
            .send_turn(
                conversation_id,
                ConversationTurnRequest {
                    project_path: self.project.clone(),
                    message: "Hello".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        conversations
            .prepare_agent_turn_request(&prepared.unwrap())
            .unwrap()
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
    harness
        .roles
        .start(&harness.project, COORDINATOR_ROLE)
        .unwrap();
    let role = frame(&harness.request(0));
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
    assert!(role.contains(".spark/state/memory/coordinator"));
    harness.finish_turn(COORDINATOR_ROLE);
    let chat = frame(&harness.user_turn("chat-1"));
    assert!(
        chat.starts_with("You are the Spark workspace assistant."),
        "{chat}"
    );
    assert!(!chat.contains(NEVER_CHANGE), "{chat}");
    let missions = crate::missions(harness.settings.clone());
    let mission = missions
        .create(
            &harness.project,
            serde_json::from_value(json!({"fields": {"title": "Ship"}})).unwrap(),
        )
        .unwrap();
    missions.start(&harness.project, &mission.id).unwrap();
    let mission_frame = frame(&harness.request(1));
    assert!(mission_frame.contains(&format!("Mission ID: {}", mission.id)));
    assert!(!mission_frame.contains(NEVER_CHANGE));
    // Every frame points flow and role authors at the bundled guide, and
    // none gives a named role special authority or memory.
    for frame in [&role, &chat, &mission_frame] {
        assert!(
            frame.contains(
                "read $SPARK_HOME/bundled/guides/flow-definition-authoring.md and follow its writing principles"
            ),
            "{frame}"
        );
        assert!(frame.contains("Never edit $SPARK_HOME/bundled."), "{frame}");
    }
    for frame in [&chat, &mission_frame] {
        for named in [
            "Coordinator",
            "coordinator",
            "Chief of Staff",
            "chief-of-staff",
        ] {
            assert!(!frame.contains(named), "{named}: {frame}");
        }
    }
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
        COORDINATOR_ROLE,
        &COORDINATOR_FILE.replacen("---\n", "---\nexecution:\n  llm_model: role-model\n", 1),
    );
    harness
        .roles
        .start(&harness.project, COORDINATOR_ROLE)
        .unwrap();
    assert_eq!(harness.request(0).model.as_deref(), Some("role-model"));
    harness.finish_turn(COORDINATOR_ROLE);
    let conversations = harness.conversations();
    let models = |id: &str| {
        conversations
            .get_snapshot(id, Some(&harness.project))
            .unwrap()["settings"]["models"]
            .clone()
    };
    assert_eq!(models("chat-1")["effective"]["model"], "project-model");
    assert_eq!(models("chat-1")["source"], "project");
    let coordinator = harness.get(COORDINATOR_ROLE).conversation_id;
    assert_eq!(models(&coordinator)["effective"]["model"], "role-model");
    assert_eq!(models(&coordinator)["source"], "role");
    // The role keeps the project's provider when it names only a model.
    assert_eq!(models(&coordinator)["effective"]["provider"], "codex");
    harness.message(COORDINATOR_ROLE, "One");
    assert_eq!(harness.request(1).model.as_deref(), Some("role-model"));
    harness.finish_turn(COORDINATOR_ROLE);
    // Role state changes re-deliver the conversation row; the settings
    // revision a save names stays that of the last committed change.
    let revision = models(&coordinator)["revision"]
        .as_str()
        .unwrap()
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
    harness.message(COORDINATOR_ROLE, "Two");
    assert_eq!(
        harness.request(2).model.as_deref(),
        Some("conversation-model")
    );
    harness.finish_turn(COORDINATOR_ROLE);
}

#[test]
fn no_role_exists_until_it_is_started_and_started_roles_pin_by_name() {
    let harness = Harness::new();
    let root = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .ensure_project_paths(&harness.project)
        .unwrap()
        .root;
    let repository =
        spark_storage::ConversationRepository::new(crate::shared_store(&harness.settings.data_dir));
    let role_threads = || {
        harness
            .conversations()
            .list_project_conversations(&harness.project)
            .unwrap()
            .into_iter()
            .filter_map(|thread| thread.standing_role.map(|role| role.name))
            .collect::<Vec<_>>()
    };
    // Reads write nothing and find no role, whatever its name.
    assert!(harness.roles.list(&harness.project).unwrap().is_empty());
    for name in [COORDINATOR_ROLE, CHIEF_OF_STAFF_ROLE] {
        assert!(matches!(
            harness.roles.get(&harness.project, name),
            Err(WorkspaceError::NotFound(_))
        ));
    }
    assert!(!root.join("roles").exists());
    assert!(role_threads().is_empty());
    assert!(matches!(
        harness.roles.process(&harness.project, COORDINATOR_ROLE),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(!root.join("roles/coordinator.json").exists());
    let project_id = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .project_paths(&harness.project)
        .unwrap()
        .project_id;
    let coordinator = spark_workspace::roles::conversation_id(&project_id, COORDINATOR_ROLE);
    // A trigger does not start a role either.
    let delivered = harness
        .roles
        .deliver_trigger(&spark_triggers::TriggerActivationRequest {
            trigger_id: "trigger-nightly".into(),
            project_path: harness.project.clone(),
            trigger_name: "Nightly".into(),
            source_type: "schedule".into(),
            action: serde_json::from_value(
                json!({"mode": "role", "role": COORDINATOR_ROLE, "static_context": {}}),
            )
            .unwrap(),
            source_payload: json!({}),
            delivery_id: None,
        });
    assert!(
        matches!(delivered, Err(WorkspaceError::NotFound(_))),
        "{delivered:?}"
    );
    assert!(repository
        .conversation_project_path(&coordinator)
        .unwrap()
        .is_none());

    // Starting saves the record and creates its conversation.
    harness.start(CHIEF_OF_STAFF_ROLE);
    assert!(root.join("roles/chief-of-staff.json").exists());
    assert_eq!(role_threads(), [CHIEF_OF_STAFF_ROLE]);
    // A saved record whose conversation is missing, as after an interrupted
    // start, is listed without a thread; starting it creates the conversation.
    std::fs::write(
        root.join("roles/coordinator.json"),
        json!({"name": COORDINATOR_ROLE, "project_id": project_id, "project_path": harness.project})
            .to_string(),
    )
    .unwrap();
    let names = || {
        harness
            .roles
            .list(&harness.project)
            .unwrap()
            .into_iter()
            .map(|role| role.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(), [CHIEF_OF_STAFF_ROLE, COORDINATOR_ROLE]);
    assert_eq!(role_threads(), [CHIEF_OF_STAFF_ROLE]);
    let restarted = harness
        .roles
        .start(&harness.project, COORDINATOR_ROLE)
        .unwrap();
    assert_eq!(restarted.conversation_id, coordinator);
    assert!(restarted.conversation_handle.is_some());
    // Role threads pin above newer ordinary threads, sorted by name rather
    // than recency.
    crate::write_conversation_snapshot(
        &harness.settings.data_dir,
        &json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 0,
            "conversation_id": "chat-latest",
            "conversation_handle": "brisk-bank",
            "project_path": harness.project,
            "title": "Latest",
            "created_at": "2099-01-01T00:00:00Z",
            "updated_at": "2099-01-01T00:00:00Z",
            "turns": []
        }),
    );
    let threads = harness
        .conversations()
        .list_project_conversations(&harness.project)
        .unwrap();
    assert_eq!(role_threads(), [CHIEF_OF_STAFF_ROLE, COORDINATOR_ROLE]);
    assert!(threads[..2]
        .iter()
        .all(|thread| thread.standing_role.is_some()));
    assert_eq!(threads[2].conversation_id, "chat-latest");
}

#[test]
fn a_run_the_role_launched_reports_back_as_a_turn() {
    let harness = Harness::new();
    crate::install_named_flow(&harness.settings, "work/ok", tool_flow("true"));
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    let begin = harness.request(0);
    assert_eq!(
        begin.prompt,
        "Begin your work as this project's standing role."
    );
    assert!(frame(&begin).contains("You are this project's Chief of Staff."));
    let run = harness
        .launch(CHIEF_OF_STAFF_ROLE, "work/ok", "Build it")
        .unwrap();
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let record = store
        .read_run_record(&store.find_run_root(&run).unwrap().unwrap())
        .unwrap()
        .unwrap();
    let role = harness.get(CHIEF_OF_STAFF_ROLE);
    assert_eq!(
        json!(record.launch_context)["context.spark_owner"],
        json!({"conversation_id": role.conversation_id})
    );
    // Its run is in flight, so the role stays working after the turn ends.
    harness.agent.release(1);
    wait_terminal(&harness.settings, &run);
    crate::deliver_run_events(&harness.settings, &run);
    assert_eq!(
        crate::batch_events(&harness.request(1).prompt),
        [format!(
            "Run {run} (work/ok, \"Build it\") ended completed."
        )]
    );
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    assert_eq!(
        harness.get(CHIEF_OF_STAFF_ROLE).inbox.runs[0].status,
        "completed"
    );
}

#[test]
fn a_stopped_role_records_events_and_delivers_them_when_started_again() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    let stopped = harness
        .roles
        .stop(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    assert_eq!(stopped.state, RoleState::Stopped);
    let recorded = harness.message(CHIEF_OF_STAFF_ROLE, "While you were out");
    assert_eq!(recorded.inbox.event_seq, recorded.inbox.cursor + 1);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 1);
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    assert_eq!(
        crate::batch_events(&harness.request(1).prompt),
        [harness.sent("While you were out")]
    );
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
}

#[test]
fn launches_over_the_role_file_budget_are_refused() {
    let harness = Harness::new();
    crate::install_named_flow(&harness.settings, "work/ok", tool_flow("true"));
    // The org copy's budget replaces the bundled file's defaults.
    let budget = |concurrent: usize| {
        harness.org_role(
            CHIEF_OF_STAFF_ROLE,
            &CHIEF_OF_STAFF_FILE.replacen(
                "standing: true",
                &format!("standing: true\nbudget:\n  concurrent_runs: {concurrent}"),
                1,
            ),
        )
    };
    budget(0);
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    assert!(
        frame(&harness.request(0)).contains("budget (0 concurrent runs)"),
        "{}",
        frame(&harness.request(0))
    );
    let refused = harness
        .launch(CHIEF_OF_STAFF_ROLE, "work/ok", "One")
        .unwrap_err();
    assert!(
        refused.to_string().contains("concurrent_runs (0)")
            && refused.to_string().contains("roles/chief-of-staff.md"),
        "{refused}"
    );
    let root = spark_storage::ProjectRegistry::new(&harness.settings.data_dir)
        .ensure_project_paths(&harness.project)
        .unwrap()
        .root;
    let record = spark_storage::workspace_records::RecordRepository::roles(&root)
        .read(CHIEF_OF_STAFF_ROLE)
        .unwrap()
        .unwrap();
    // The record holds no budget and no inbox, only whether it is stopped.
    for key in ["budget", "runs", "cursor", "event_seq"] {
        assert!(record.get(key).is_none(), "{record}");
    }
    let role = harness.get(CHIEF_OF_STAFF_ROLE);
    assert!(role.inbox.runs.is_empty());
    assert_eq!(role.budget.concurrent_runs, 0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
}

#[test]
fn only_a_role_file_declaring_standing_can_start() {
    let harness = Harness::new();
    harness.org_role("helper", "---\ntype: Role\n---\n\nYou help.\n");
    let refused = harness.roles.start(&harness.project, "helper").unwrap_err();
    assert!(
        matches!(&refused, WorkspaceError::Validation(message) if message.contains("standing: true")),
        "{refused}"
    );
    assert!(harness.roles.get(&harness.project, "helper").is_err());
    // No name is exempt: a coordinator file without the flag cannot start.
    harness.org_role(
        COORDINATOR_ROLE,
        "---\ntype: Role\n---\n\nYou coordinate.\n",
    );
    let refused = harness
        .roles
        .start(&harness.project, COORDINATOR_ROLE)
        .unwrap_err();
    assert!(
        matches!(&refused, WorkspaceError::Validation(message) if message.contains("standing: true")),
        "{refused}"
    );
    assert!(harness
        .roles
        .get(&harness.project, COORDINATOR_ROLE)
        .is_err());
    // Any role file declaring it starts, with its file's budget.
    harness.org_role(
        "scout",
        "---\ntype: Role\nstanding: true\n---\n\nYou scout.\n",
    );
    let scout = harness.roles.start(&harness.project, "scout").unwrap();
    assert_eq!(scout.budget.concurrent_runs, 4);
    harness.request(0);
    harness.finish_turn("scout");
}

#[test]
fn a_message_from_another_role_names_its_sender() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    let handle = harness.handle(CHIEF_OF_STAFF_ROLE);
    let send = |to: &str, sender: &str| {
        spark_workspace::inbox::send_message(
            &harness.settings,
            &crate::shared_store(&harness.settings.data_dir),
            to,
            ThreadMessageRequest {
                conversation_handle: sender.into(),
                message: " Fixed the flow ".into(),
            },
        )
    };
    harness.start(COORDINATOR_ROLE);
    harness.message(COORDINATOR_ROLE, "Hello");
    harness.request(2);
    harness.finish_turn(COORDINATOR_ROLE);
    send(&harness.handle(COORDINATOR_ROLE), &handle).unwrap();
    let request = harness.request(3);
    assert_eq!(
        crate::batch_events(&request.prompt),
        [format!(
            "Message from conversation {handle} (standing role chief-of-staff), project {}:\nFixed the flow",
            harness.project
        )]
    );
    assert!(
        request.prompt.contains(&format!("from role-")),
        "{}",
        request.prompt
    );
    assert_eq!(
        request.conversation_id,
        harness.get(COORDINATOR_ROLE).conversation_id
    );
    harness.finish_turn(COORDINATOR_ROLE);
    assert!(matches!(
        send("no-such-handle", &handle),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        send(&harness.handle(COORDINATOR_ROLE), "no-such-handle"),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn a_changed_persona_is_announced_once() {
    let harness = Harness::new();
    harness.start(COORDINATOR_ROLE);
    harness.message(COORDINATOR_ROLE, "One");
    assert_eq!(
        crate::batch_events(&harness.request(1).prompt),
        [harness.sent("One")]
    );
    harness.finish_turn(COORDINATOR_ROLE);
    harness.org_role(
        COORDINATOR_ROLE,
        "---\ntype: Role\ntitle: Coordinator\n---\n\nYou are a terse coordinator.\n",
    );
    harness.message(COORDINATOR_ROLE, "Two");
    let changed = harness.request(2);
    assert!(
        changed
            .prompt
            .starts_with("Your instructions changed since your last turn"),
        "{}",
        changed.prompt
    );
    assert!(changed.prompt.ends_with(&harness.sent("Two")));
    assert!(frame(&changed).starts_with("You are a terse coordinator."));
    harness.finish_turn(COORDINATOR_ROLE);
    harness.message(COORDINATOR_ROLE, "Three");
    assert_eq!(
        crate::batch_events(&harness.request(3).prompt),
        [harness.sent("Three")]
    );
    harness.finish_turn(COORDINATOR_ROLE);
}

#[test]
fn recovery_delivers_a_role_runs_missing_terminal_event() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    // A run failed before a restart while its result was still pending.
    let conversation = harness.get(CHIEF_OF_STAFF_ROLE).conversation_id;
    crate::owned_run(
        &harness.settings,
        &harness.project,
        &conversation,
        "run-lost",
        "failed",
    );
    assert!(!crate::deliver_run_events(&harness.settings, "run-lost"));
    crate::recover(&harness.settings);
    assert_eq!(
        crate::batch_events(&harness.request(1).prompt),
        ["Run run-lost (work/seeded, \"Seeded work\") ended failed."]
    );
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    assert_eq!(
        harness.get(CHIEF_OF_STAFF_ROLE).inbox.runs[0].status,
        "failed"
    );
    crate::recover(&harness.settings);
    assert_eq!(harness.agent.requests.lock().unwrap().len(), 2);
}

#[test]
fn a_question_from_a_run_the_role_launched_arrives_as_a_turn_with_answer_instructions() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    crate::owned_run(
        &harness.settings,
        &harness.project,
        &harness.get(CHIEF_OF_STAFF_ROLE).conversation_id,
        "run-asks",
        "running",
    );
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let paths = store.find_run_root("run-asks").unwrap().unwrap();
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
    crate::deliver_run_events(&harness.settings, "run-asks");
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
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    let conversation = harness.get(CHIEF_OF_STAFF_ROLE).conversation_id;
    let repository =
        spark_storage::ConversationRepository::new(crate::shared_store(&harness.settings.data_dir));
    for index in 0..50 {
        let run = format!("run-done-{index}");
        repository
            .register_inbox_run(&conversation, &run, "work/seeded", "Seeded work")
            .unwrap();
        repository
            .set_inbox_run_status(&conversation, &run, "completed")
            .unwrap();
        if index == 10 {
            repository
                .register_inbox_run(&conversation, "run-other", "work/seeded", "Seeded work")
                .unwrap();
        }
    }
    crate::owned_run(
        &harness.settings,
        &harness.project,
        &conversation,
        "run-lost",
        "failed",
    );
    // Recovery reconciles the run to its terminal status, which trims.
    crate::recover(&harness.settings);
    harness.request(1);
    let runs = harness.get(CHIEF_OF_STAFF_ROLE).inbox.runs;
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
fn restarts_keep_trimmed_runs_out_of_the_roster_and_restore_missing_ones_in_launch_order() {
    let harness = Harness::new();
    harness
        .roles
        .start(&harness.project, CHIEF_OF_STAFF_ROLE)
        .unwrap();
    harness.request(0);
    harness.finish_turn(CHIEF_OF_STAFF_ROLE);
    harness.agent.release(1000);
    let conversation = harness.get(CHIEF_OF_STAFF_ROLE).conversation_id;
    let repository =
        spark_storage::ConversationRepository::new(crate::shared_store(&harness.settings.data_dir));
    let store = attractor_runtime::RunStore::for_settings(&harness.settings);
    let launch = |run_id: &str| {
        let mut record = attractor_core::RunRecord::new(run_id, &harness.project);
        record.flow_name = "work/seeded".into();
        record.status = "running".into();
        record.started_at = attractor_runtime::utc_timestamp();
        record.launch_context = Some(
            [(
                "context.spark_owner".to_string(),
                json!({ "conversation_id": conversation }),
            )]
            .into(),
        );
        store
            .create_run(attractor_runtime::CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap()
    };
    // 52 runs launched from the role finish and are reported; the roster
    // keeps the newest 50.
    let mut lost_started_at = String::new();
    for index in 0..52 {
        if index == 30 {
            lost_started_at = attractor_runtime::utc_timestamp();
        }
        let run = format!("run-{index:02}");
        let paths = launch(&run);
        repository
            .register_inbox_run(&conversation, &run, "work/seeded", "Seeded work")
            .unwrap();
        let mut record = store.read_run_record(&paths).unwrap().unwrap();
        record.status = "completed".into();
        store.write_run_record(&paths, &record).unwrap();
        store
            .write_result(
                &paths,
                &attractor_core::RunResult::unavailable(&run, "completed"),
            )
            .unwrap();
        assert!(crate::deliver_run_events(&harness.settings, &run));
    }
    wait_for("every report consumed", || {
        repository
            .pending_inbox_events(&conversation)
            .unwrap()
            .is_empty()
            && harness.get(CHIEF_OF_STAFF_ROLE).state != RoleState::Working
    });
    let roster = |repository: &spark_storage::ConversationRepository| {
        repository
            .inbox_runs(&conversation)
            .unwrap()
            .into_iter()
            .map(|run| run.run_id)
            .collect::<Vec<_>>()
    };
    let kept: Vec<String> = (2..52).map(|index| format!("run-{index:02}")).collect();
    assert_eq!(roster(&repository), kept);
    // A run launched between run-29 and run-30 that a crash left out of the
    // roster, still running.
    let lost = launch("run-lost");
    let mut record = store.read_run_record(&lost).unwrap().unwrap();
    record.started_at = lost_started_at;
    store.write_run_record(&lost, &record).unwrap();
    let turns = harness.agent.requests.lock().unwrap().len();
    let mut expected = kept.clone();
    expected.insert(28, "run-lost".to_string());
    for _ in 0..2 {
        crate::recover(&harness.settings);
        assert_eq!(roster(&repository), expected);
        assert_eq!(repository.inbox_events(&conversation).unwrap().len(), 52);
        assert_eq!(harness.agent.requests.lock().unwrap().len(), turns);
    }
    assert!(harness
        .get(CHIEF_OF_STAFF_ROLE)
        .inbox
        .runs
        .iter()
        .any(|run| run.run_id == "run-lost" && run.in_flight()));
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
        crate::batch_events(&harness.request(1).prompt),
        [harness.sent("Cut the release")]
    );
    harness.finish_turn(name);
    for unsafe_name in ["../outside", ".hidden", "a/b"] {
        assert!(harness.roles.start(&harness.project, unsafe_name).is_err());
        assert!(harness.roles.get(&harness.project, unsafe_name).is_err());
    }
}

#[test]
fn a_roles_first_turn_makes_its_memory_ignored_and_again_after_state_is_deleted() {
    let harness = Harness::new();
    let project = std::path::Path::new(&harness.project);
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(project)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    git(&["init", "-q"]);
    // A project copy of the role: definitions stay visible to Git.
    harness.org_role(COORDINATOR_ROLE, COORDINATOR_FILE);
    let state = project.join(".spark/state");
    let memory = state.join("memory/coordinator");
    harness.start(COORDINATOR_ROLE);
    let _ = std::fs::remove_dir_all(&state);
    for (turn, pass) in ["fresh", "after state deletion"].into_iter().enumerate() {
        assert!(!state.exists(), "{pass}");
        harness.message(COORDINATOR_ROLE, "Note what you know.");
        let role = frame(&harness.request(turn + 1));
        assert!(role.contains(&memory.display().to_string()), "{pass}");
        assert_eq!(
            std::fs::read_to_string(state.join(".gitignore")).unwrap(),
            "*\n",
            "{pass}"
        );
        // The memory the frame tells the role to create.
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("index.md"), "---\ntype: Memory\n---\n").unwrap();
        let status = git(&["status", "--porcelain", "--untracked-files=all"]);
        assert!(!status.contains(".spark/state"), "{pass}: {status}");
        assert!(
            status.contains("?? .spark/definitions/roles/coordinator.md"),
            "{pass}: {status}"
        );
        harness.finish_turn(COORDINATOR_ROLE);
        std::fs::remove_dir_all(&state).unwrap();
    }
}
