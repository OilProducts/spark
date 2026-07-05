use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;
use spark_agent_adapter::{
    build_environment_context_block, build_provider_base_instructions, build_system_prompt,
    build_system_prompt_with_user_overrides, build_tool_descriptions, create_anthropic_profile,
    create_gemini_profile, create_openai_compatible_profile, create_openai_profile,
    discover_project_documents, discover_project_documents_with_budget, load_project_documents,
    render_project_documents, snapshot_environment_context, CommandOptions, DirEntry,
    EnvironmentError, EnvironmentResult, ExecResult, ExecutionEnvironment,
    ExecutionEnvironmentBackend, GrepOptions, ProviderProfile, Session, SessionConfig,
    PROJECT_INSTRUCTION_BYTE_BUDGET, PROJECT_INSTRUCTION_TRUNCATION_MARKER,
};
use tempfile::TempDir;
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, FinishReason, Message, ProviderAdapter, Request,
    Response, StreamEvents, Tool,
};

#[test]
fn layered_prompt_orders_context_tools_project_documents_and_user_overrides() {
    let workspace = TempDir::new().expect("workspace");
    let root = workspace.path();
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("nested dir");
    let environment = ExecutionEnvironment::local(&nested);
    let branch = initialize_git_repo(&environment, root);
    environment
        .write_file(root.join("tracked.txt"), "tracked\nmodified\n")
        .expect("modify tracked file");
    environment
        .write_file(nested.join("untracked.txt"), "new\n")
        .expect("untracked file");
    write_instruction_documents(&environment, root, &nested);

    let mut profile = create_openai_profile("gpt-5.2");
    profile.knowledge_cutoff = Some("2024-06".to_string());
    profile.register_tool(
        Tool::passive_with_schema(
            "lookup",
            Some("Lookup values".to_string()),
            Some(json!({"type": "object"})),
        )
        .expect("lookup tool"),
    );

    let context = snapshot_environment_context(&profile, &environment);
    let prompt = build_system_prompt_with_user_overrides(
        &profile,
        &environment,
        Some("User override guidance"),
    );
    let provider_block = build_provider_base_instructions(&profile);
    let environment_block = build_environment_context_block(&context);
    let tools_block = build_tool_descriptions(&profile);
    let project_block = load_project_documents(&environment, &profile);

    assert_eq!(context.working_directory, nested.to_string_lossy());
    assert!(context.is_git_repository);
    assert_eq!(context.current_branch, branch);
    assert_eq!(context.modified_count, 1);
    assert!(context.untracked_count >= 1);
    assert_eq!(context.recent_commit_messages, ["Initial commit"]);
    assert_eq!(context.platform, environment.platform());
    assert_eq!(context.os_version, environment.os_version());
    assert_eq!(
        context.today,
        time::OffsetDateTime::now_utc().date().to_string()
    );
    assert_eq!(context.model_display_name, "GPT-5.2");
    assert_eq!(context.knowledge_cutoff, "2024-06");

    assert_eq!(prompt.find(&provider_block), Some(0));
    assert!(index_of(&prompt, &environment_block) > index_of(&prompt, &provider_block));
    assert!(index_of(&prompt, &tools_block) > index_of(&prompt, &environment_block));
    assert!(index_of(&prompt, &project_block) > index_of(&prompt, &tools_block));
    assert!(index_of(&prompt, "User override guidance") > index_of(&prompt, &project_block));

    assert!(tools_block.contains("lookup"));
    assert!(tools_block.contains("Lookup values"));
    assert!(project_block.contains("root agents"));
    assert!(project_block.contains("nested agents"));
    assert!(project_block.contains("root openai"));
    assert!(project_block.contains("nested openai"));
    assert!(!project_block.contains("root claude"));
    assert!(!project_block.contains("nested claude"));
    assert!(!project_block.contains("root gemini"));
    assert!(!project_block.contains("nested gemini"));
}

#[test]
fn project_document_discovery_filters_provider_docs_and_keeps_root_to_leaf_order() {
    let workspace = TempDir::new().expect("workspace");
    let root = workspace.path();
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("nested dir");
    let environment = ExecutionEnvironment::local(&nested);
    initialize_git_repo(&environment, root);
    write_instruction_documents(&environment, root, &nested);

    let cases = [
        (
            create_openai_profile("gpt-5.2"),
            vec![
                "AGENTS.md",
                ".codex/instructions.md",
                "nested/AGENTS.md",
                "nested/.codex/instructions.md",
            ],
            vec![
                "root agents",
                "root openai",
                "nested agents",
                "nested openai",
            ],
        ),
        (
            create_anthropic_profile("claude-sonnet-4-5"),
            vec![
                "AGENTS.md",
                "CLAUDE.md",
                "nested/AGENTS.md",
                "nested/CLAUDE.md",
            ],
            vec![
                "root agents",
                "root claude",
                "nested agents",
                "nested claude",
            ],
        ),
        (
            create_gemini_profile("gemini-3.1-pro-preview"),
            vec![
                "AGENTS.md",
                "GEMINI.md",
                "nested/AGENTS.md",
                "nested/GEMINI.md",
            ],
            vec![
                "root agents",
                "root gemini",
                "nested agents",
                "nested gemini",
            ],
        ),
    ];

    for (profile, expected_paths, expected_contents) in cases {
        let bundle = discover_project_documents(&environment, &profile);
        assert_eq!(
            bundle
                .documents
                .iter()
                .map(|document| document.path.as_str())
                .collect::<Vec<_>>(),
            expected_paths
        );
        assert_eq!(
            bundle
                .documents
                .iter()
                .map(|document| document.content.as_str())
                .collect::<Vec<_>>(),
            expected_contents
        );
        assert!(!bundle.truncated);
    }
}

#[test]
fn openai_compatible_project_documents_use_profile_identity_over_model_provider() {
    let workspace = TempDir::new().expect("workspace");
    let root = workspace.path();
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("nested dir");
    let environment = ExecutionEnvironment::local(&nested);
    initialize_git_repo(&environment, root);
    write_instruction_documents(&environment, root, &nested);

    let profile = create_openai_compatible_profile("openrouter", "anthropic/claude-sonnet-4.5");
    let bundle = discover_project_documents(&environment, &profile);
    let prompt = build_system_prompt(&profile, &environment);

    assert_eq!(
        bundle
            .documents
            .iter()
            .map(|document| document.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "AGENTS.md",
            ".codex/instructions.md",
            "nested/AGENTS.md",
            "nested/.codex/instructions.md",
        ]
    );
    assert!(prompt.contains("root agents"));
    assert!(prompt.contains("nested agents"));
    assert!(prompt.contains("root openai"));
    assert!(prompt.contains("nested openai"));
    assert!(prompt.contains(".codex/instructions.md"));
    assert!(!prompt.contains("root claude"));
    assert!(!prompt.contains("nested claude"));
    assert!(!prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains("root gemini"));
    assert!(!prompt.contains("nested gemini"));
    assert!(!prompt.contains("GEMINI.md"));
}

#[test]
fn project_document_rendering_appends_truncation_marker_when_budget_overflows() {
    let workspace = TempDir::new().expect("workspace");
    let root = workspace.path();
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("nested dir");
    let environment = ExecutionEnvironment::local(&nested);
    initialize_git_repo(&environment, root);

    environment
        .write_file(
            root.join("AGENTS.md"),
            &format!("intro\n{}\nroot-end", "A".repeat(33_000)),
        )
        .expect("root agents");
    environment
        .write_file(nested.join("AGENTS.md"), "nested guidance")
        .expect("nested agents");

    let profile = create_openai_profile("gpt-5.2");
    let bundle = discover_project_documents_with_budget(
        &environment,
        &profile,
        PROJECT_INSTRUCTION_BYTE_BUDGET,
    );
    let rendered = render_project_documents(&bundle);

    assert!(bundle.truncated);
    assert_eq!(bundle.len(), 1);
    assert_eq!(bundle.documents[0].path, "AGENTS.md");
    assert!(bundle.documents[0].truncated);
    assert!(rendered.contains(PROJECT_INSTRUCTION_TRUNCATION_MARKER));
    assert!(rendered.starts_with("### AGENTS.md\nintro"));
    assert!(!rendered.contains("root-end"));
    assert!(!rendered.contains("nested guidance"));
}

#[test]
fn session_uses_the_system_prompt_snapshotted_at_session_start() {
    let backend = Arc::new(MutableWorkingDirectoryBackend::new(
        "initial-working-directory",
    ));
    let environment =
        ExecutionEnvironment::from_backend_arc(backend.clone(), Some(PathBuf::from("initial")));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("fake-provider", calls.clone()));
    let client = Client::from_adapters(vec![adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.display_name = Some("Session Model".to_string());
    let mut session = Session::new(profile.clone(), environment, SessionConfig::default());
    let expected_prompt = build_system_prompt(
        &profile,
        &ExecutionEnvironment::from_backend_arc(
            Arc::new(MutableWorkingDirectoryBackend::new(
                "initial-working-directory",
            )),
            Some(PathBuf::from("initial")),
        ),
    );
    backend.set_working_directory("mutated-working-directory");

    session
        .process_input(&client, "Hello")
        .expect("process input");

    let requests = calls.lock().expect("calls");
    let system_prompt = requests[0].messages[0].text();
    assert_eq!(system_prompt, expected_prompt);
    assert_eq!(system_prompt, session.system_prompt_snapshot);
    assert!(system_prompt.contains("initial-working-directory"));
    assert!(!system_prompt.contains("mutated-working-directory"));
}

fn initialize_git_repo(environment: &ExecutionEnvironment, root: &Path) -> String {
    assert_eq!(
        environment
            .exec_command(
                "git init",
                CommandOptions {
                    working_dir: Some(root.to_path_buf()),
                    ..CommandOptions::default()
                },
            )
            .expect("git init")
            .exit_code,
        0
    );
    assert_eq!(
        environment
            .exec_command(
                "git config user.name \"Test User\"",
                CommandOptions {
                    working_dir: Some(root.to_path_buf()),
                    ..CommandOptions::default()
                },
            )
            .expect("git config user.name")
            .exit_code,
        0
    );
    assert_eq!(
        environment
            .exec_command(
                "git config user.email \"test@example.com\"",
                CommandOptions {
                    working_dir: Some(root.to_path_buf()),
                    ..CommandOptions::default()
                },
            )
            .expect("git config user.email")
            .exit_code,
        0
    );
    environment
        .write_file(root.join("tracked.txt"), "tracked\n")
        .expect("tracked file");
    assert_eq!(
        environment
            .exec_command(
                "git add tracked.txt",
                CommandOptions {
                    working_dir: Some(root.to_path_buf()),
                    ..CommandOptions::default()
                },
            )
            .expect("git add")
            .exit_code,
        0
    );
    assert_eq!(
        environment
            .exec_command(
                "git commit -m \"Initial commit\"",
                CommandOptions {
                    working_dir: Some(root.to_path_buf()),
                    ..CommandOptions::default()
                },
            )
            .expect("git commit")
            .exit_code,
        0
    );
    environment
        .exec_command(
            "git branch --show-current",
            CommandOptions {
                working_dir: Some(root.to_path_buf()),
                ..CommandOptions::default()
            },
        )
        .expect("git branch")
        .stdout
        .trim()
        .to_string()
}

fn write_instruction_documents(environment: &ExecutionEnvironment, root: &Path, nested: &Path) {
    for (path, content) in [
        (root.join("AGENTS.md"), "root agents"),
        (root.join(".codex/instructions.md"), "root openai"),
        (root.join("CLAUDE.md"), "root claude"),
        (root.join("GEMINI.md"), "root gemini"),
        (nested.join("AGENTS.md"), "nested agents"),
        (nested.join(".codex/instructions.md"), "nested openai"),
        (nested.join("CLAUDE.md"), "nested claude"),
        (nested.join("GEMINI.md"), "nested gemini"),
    ] {
        environment.write_file(path, content).expect("write doc");
    }
}

fn index_of(haystack: &str, needle: &str) -> usize {
    haystack.find(needle).expect("prompt layer")
}

#[derive(Debug)]
struct MutableWorkingDirectoryBackend {
    working_directory: Mutex<String>,
}

impl MutableWorkingDirectoryBackend {
    fn new(working_directory: impl Into<String>) -> Self {
        Self {
            working_directory: Mutex::new(working_directory.into()),
        }
    }

    fn set_working_directory(&self, working_directory: impl Into<String>) {
        *self.working_directory.lock().expect("working directory") = working_directory.into();
    }
}

impl ExecutionEnvironmentBackend for MutableWorkingDirectoryBackend {
    fn read_file(
        &self,
        path: &Path,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        Err(EnvironmentError::FileNotFound(path.to_path_buf()))
    }

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>> {
        Err(EnvironmentError::FileNotFound(path.to_path_buf()))
    }

    fn write_file(&self, _path: &Path, _content: &str) -> EnvironmentResult<()> {
        Ok(())
    }

    fn file_exists(&self, _path: &Path) -> bool {
        false
    }

    fn is_directory(&self, _path: &Path) -> bool {
        false
    }

    fn delete_file(&self, _path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn rename_file(&self, _source_path: &Path, _destination_path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn list_directory(&self, _path: &Path, _depth: usize) -> EnvironmentResult<Vec<DirEntry>> {
        Ok(Vec::new())
    }

    fn exec_command(
        &self,
        _command: &str,
        _options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        Ok(ExecResult {
            exit_code: 1,
            duration_ms: 1,
            ..ExecResult::default()
        })
    }

    fn grep(
        &self,
        _pattern: &str,
        _path: &Path,
        _options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        Ok(String::new())
    }

    fn glob(&self, _pattern: &str, _path: &Path) -> EnvironmentResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn initialize(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn cleanup(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn working_directory(&self) -> String {
        self.working_directory
            .lock()
            .expect("working directory")
            .clone()
    }

    fn platform(&self) -> String {
        "test-platform".to_string()
    }

    fn os_version(&self) -> String {
        "test-os".to_string()
    }
}

struct RecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl RecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model,
            provider: request.provider.unwrap_or_default(),
            message: Message::assistant("Done"),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "stream not supported",
        ))
    }
}
