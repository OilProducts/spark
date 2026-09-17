use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use attractor_core::{FailureKind, Outcome, OutcomeStatus};
use attractor_runtime::{
    NodeExecutionRequest, NodeExecutor, RuntimeHandlerRunner, RuntimeNodeError,
};
use serde_json::Value;

use crate::modes::ExecutionMode;
use crate::profile::ExecutionProfileSelection;
use crate::protocol::{outcome_from_payload, RunRootMetadata, WorkerFrame, WorkerNodeRequest};

#[derive(Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: String,
    pub env: BTreeMap<String, String>,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("environment_keys", &self.env.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl CommandSpec {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            stdin: String::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait ContainerCommandRunner: Send {
    fn command_exists(&self, program: &str) -> bool;
    fn run(&mut self, spec: CommandSpec) -> std::io::Result<CommandResult>;
    fn run_streaming(
        &mut self,
        spec: CommandSpec,
        on_stdout_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<CommandResult> {
        let result = self.run(spec)?;
        for line in result.stdout.lines() {
            on_stdout_line(line);
        }
        Ok(result)
    }
}

#[derive(Debug, Default)]
pub struct SystemCommandRunner;

impl ContainerCommandRunner for SystemCommandRunner {
    fn command_exists(&self, program: &str) -> bool {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths).find_map(|path| {
                    let candidate = path.join(program);
                    candidate.is_file().then_some(())
                })
            })
            .is_some()
    }

    fn run(&mut self, spec: CommandSpec) -> std::io::Result<CommandResult> {
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        command.envs(&spec.env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if !spec.stdin.is_empty() {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(spec.stdin.as_bytes())?;
            }
        }
        let output = child.wait_with_output()?;
        Ok(CommandResult {
            exit_code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_streaming(
        &mut self,
        spec: CommandSpec,
        on_stdout_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<CommandResult> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(&spec.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(spec.stdin.as_bytes())?;
        }
        let stderr = child.stderr.take().expect("piped stderr");
        let stderr_thread = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = BufReader::new(stderr).read_to_end(&mut bytes);
            bytes
        });
        let mut stdout_text = String::new();
        let stdout = child.stdout.take().expect("piped stdout");
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            stdout_text.push_str(&line);
            stdout_text.push('\n');
            on_stdout_line(&line);
        }
        let status = child.wait()?;
        let stderr = stderr_thread.join().unwrap_or_default();
        Ok(CommandResult {
            exit_code: status.code().unwrap_or(1),
            stdout: stdout_text,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

pub struct ContainerizedNodeExecutor {
    inner: RuntimeHandlerRunner,
    selection: ExecutionProfileSelection,
    command_runner: Box<dyn ContainerCommandRunner>,
    container_id: Option<String>,
    target_native: Option<Value>,
    container_name: String,
    docker_program: String,
    last_cleanup_error: Option<String>,
    cleanup_after_execute: bool,
}

impl ContainerizedNodeExecutor {
    pub fn new(selection: ExecutionProfileSelection, inner: RuntimeHandlerRunner) -> Self {
        let container_name = format!(
            "spark-run-{}-{}",
            selection.selected_profile_id,
            unique_suffix()
        );
        Self {
            inner,
            selection,
            command_runner: Box::<SystemCommandRunner>::default(),
            container_id: None,
            target_native: None,
            container_name,
            docker_program: "docker".to_string(),
            last_cleanup_error: None,
            cleanup_after_execute: true,
        }
    }

    pub fn with_command_runner(
        mut self,
        command_runner: impl ContainerCommandRunner + 'static,
    ) -> Self {
        self.command_runner = Box::new(command_runner);
        self
    }

    pub fn with_boxed_command_runner(
        mut self,
        command_runner: Box<dyn ContainerCommandRunner>,
    ) -> Self {
        self.command_runner = command_runner;
        self
    }

    pub fn keep_container_open(mut self) -> Self {
        self.cleanup_after_execute = false;
        self
    }

    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    pub fn close(&mut self) -> Result<(), String> {
        let Some(container_id) = self.container_id.take() else {
            return Ok(());
        };
        let result = self
            .command_runner
            .run(CommandSpec::new(
                &self.docker_program,
                ["rm", "-f", container_id.as_str()],
            ))
            .map_err(|error| {
                let message = error.to_string();
                self.last_cleanup_error = Some(message.clone());
                message
            })?;
        if result.exit_code != 0 {
            let message = if result.stderr.trim().is_empty() {
                format!("docker rm -f failed with exit code {}", result.exit_code)
            } else {
                result.stderr.trim().to_string()
            };
            self.last_cleanup_error = Some(message.clone());
            return Err(message);
        }
        Ok(())
    }

    fn execute_container(
        &mut self,
        request: NodeExecutionRequest,
    ) -> Result<Outcome, RuntimeNodeError> {
        let image = self
            .selection
            .profile
            .image
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RuntimeNodeError::terminal("local_container execution profile requires image")
            })?;
        if !self.command_runner.command_exists(&self.docker_program) {
            return Err(RuntimeNodeError::terminal(
                "Container execution requires Docker, but the docker CLI was not found.",
            ));
        }
        self.ensure_container_started(&request, &image)?;
        let mut context = request.context.clone();
        self.resolve_target_paths(&mut context)?;
        let worker_request = WorkerNodeRequest {
            run_id: request.run_id.clone(),
            flow: request.flow.clone(),
            node_id: request.node_id.clone(),
            stage_index: request.stage_index,
            attempt: request.attempt,
            prompt: request.prompt.clone(),
            context,
            context_logs: Vec::new(),
            logs_root: request.run_paths.as_ref().map(|paths| paths.logs_dir()),
            working_dir: request.run_workdir.clone(),
            backend_name: Some("provider-router".to_string()),
            model: request
                .context
                .get("_attractor.runtime.launch_model")
                .and_then(Value::as_str)
                .map(str::to_string),
            config_dir: std::env::var_os("SPARK_CONFIG_DIR").map(PathBuf::from),
            run_root: request.run_paths.as_ref().map(|paths| RunRootMetadata {
                runs_dir: paths.runs_dir.clone(),
                project_id: paths.project_id.clone(),
                root: paths.root.clone(),
            }),
        };
        let stdin = serde_json::to_string(&worker_request)
            .map_err(|error| RuntimeNodeError::terminal(error.to_string()))?
            + "\n";
        let container_id = self.container_id.clone().unwrap_or_default();
        let mut spec = CommandSpec::new(
            &self.docker_program,
            [
                "exec",
                "-i",
                container_id.as_str(),
                "spark-server",
                "worker",
                "run-node",
            ],
        );
        spec.stdin = stdin;
        let mut outcome = None;
        let mut protocol_error = None;
        let inner = self.inner.clone();
        let run_id = request.run_id.clone();
        let mut saw_result = false;
        let result = self
            .command_runner
            .run_streaming(spec, &mut |line| {
                if line.trim().is_empty() || protocol_error.is_some() {
                    return;
                }
                let frame = match serde_json::from_str::<WorkerFrame>(line) {
                    Ok(frame) => frame,
                    Err(error) => {
                        protocol_error = Some(format!("invalid worker protocol frame: {error}"));
                        return;
                    }
                };
                if saw_result {
                    protocol_error = Some("worker emitted a frame after its result".to_string());
                    return;
                }
                match frame {
                    WorkerFrame::Event(_) => {
                        inner.notify_run_event(&run_id);
                    }
                    WorkerFrame::Result(frame) => {
                        saw_result = true;
                        outcome = Some(outcome_from_payload(&frame.outcome));
                    }
                    _ => protocol_error = Some("unexpected worker protocol frame".to_string()),
                }
            })
            .map_err(|error| {
                RuntimeNodeError::terminal(format!("Container node worker failed: {error}"))
            })?;
        if let Some(error) = protocol_error {
            return Err(RuntimeNodeError::terminal(format_diagnostic(
                &error, &result,
            )));
        }
        if result.exit_code != 0 {
            return Err(RuntimeNodeError::terminal(format!(
                "Container node worker failed with exit code {}: {}",
                result.exit_code,
                result.stderr.trim()
            )));
        }
        let Some(outcome) = outcome else {
            return Err(RuntimeNodeError::terminal(
                "Container node worker exited without a result payload.",
            ));
        };
        if self.cleanup_after_execute {
            let _ = self.close();
        }
        Ok(outcome)
    }

    fn resolve_target_paths(
        &mut self,
        context: &mut attractor_core::ContextMap,
    ) -> Result<(), RuntimeNodeError> {
        let Some(native) = context
            .get_mut("internal.execution_configuration_snapshot")
            .and_then(|capture| capture.pointer_mut("/agents/native"))
        else {
            return Ok(());
        };
        if self.target_native.is_none() {
            // Arguments carry authored paths only. Discovery and home expansion run in the image.
            const RESOLVE: &str = r#"
path() { case "$1" in '~') printf '%s' "$HOME" ;; '~/'*) printf '%s/%s' "$HOME" "${1#\~/}" ;; /*) printf '%s' "$1" ;; *) printf '%s/%s' "$PWD" "$1" ;; esac; }
binary() { value=$(command -v "$1" 2>/dev/null) || value=$1; case "$value" in */*) path "$value" ;; *) printf '%s' "$value" ;; esac; }
binary "${1:-${SPARK_CODEX_APP_SERVER_BIN:-codex}}"; printf '\0'
binary "${2:-${SPARK_CLAUDE_CODE_BIN:-claude}}"; printf '\0'
path "${3:-${ATTRACTOR_CODEX_RUNTIME_ROOT:-${SPARK_HOME:-$HOME/.spark}/runtime/codex}}"; printf '\0'
path "${4:-${ATTRACTOR_CODEX_SEED_DIR:-${CODEX_HOME:-$HOME/.codex}}}"; printf '\0'
path "${5:-${SPARK_CLAUDE_CODE_CONFIG_DIR:-${CLAUDE_CONFIG_DIR:-$HOME/.claude}}}"; printf '\0'
"#;
            let keys = [
                "codex_binary",
                "claude_binary",
                "codex_runtime_root",
                "codex_seed_dir",
                "claude_config_dir",
            ];
            let mut args = vec![
                "exec".into(),
                self.container_id.clone().expect("started container"),
                "sh".into(),
                "-c".into(),
                RESOLVE.into(),
                "spark-resolve-paths".into(),
            ];
            args.extend(
                keys.iter()
                    .map(|key| native[*key].as_str().unwrap_or_default().to_owned()),
            );
            let result = self
                .command_runner
                .run(CommandSpec::new(&self.docker_program, args))
                .map_err(|_| {
                    RuntimeNodeError::terminal(
                        "Unable to resolve agent paths in the execution container.",
                    )
                })?;
            let paths: Vec<_> = result.stdout.split_terminator('\0').collect();
            if result.exit_code != 0
                || paths.len() != keys.len()
                || paths.iter().any(|path| path.is_empty())
            {
                return Err(RuntimeNodeError::terminal(
                    "Unable to resolve agent paths in the execution container.",
                ));
            }
            let mut resolved = native.clone();
            for (key, path) in keys.into_iter().zip(paths) {
                resolved[key] = Value::String(path.into());
            }
            self.target_native = Some(resolved);
        }
        *native = self.target_native.clone().expect("resolved paths");
        Ok(())
    }

    fn ensure_container_started(
        &mut self,
        request: &NodeExecutionRequest,
        image: &str,
    ) -> Result<(), RuntimeNodeError> {
        if self.container_id.is_some() {
            return Ok(());
        }
        let run_root = request
            .run_paths
            .as_ref()
            .map(|paths| paths.root.clone())
            .unwrap_or_else(|| request.run_workdir.join(".spark-run"));
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            self.container_name.clone(),
            "--label".to_string(),
            format!("spark.run_id={}", request.run_id),
            "--label".to_string(),
            "spark.execution_mode=local_container".to_string(),
            "--label".to_string(),
            format!("spark.project_path={}", request.run_workdir.display()),
        ];
        let mut mounts = vec![
            mount_arg(&request.run_workdir, &request.run_workdir),
            mount_arg(
                run_root.parent().unwrap_or(run_root.as_path()),
                run_root.parent().unwrap_or(run_root.as_path()),
            ),
            mount_arg(&run_root, &run_root),
        ];
        mounts.extend(profile_mounts(&self.selection.profile)?);
        for mount in dedupe(mounts) {
            args.push("-v".to_string());
            args.push(mount);
        }
        let environment = container_env(
            &request.context,
            &serde_json::to_value(&request.flow).expect("serializable flow"),
            &spark_common::paths::ProcessEnvironment,
        );
        for key in environment.keys() {
            args.push("-e".to_string());
            args.push(key.clone());
        }
        args.extend([
            image.to_string(),
            "tail".to_string(),
            "-f".to_string(),
            "/dev/null".to_string(),
        ]);
        let mut command = CommandSpec::new(&self.docker_program, args);
        command.env = environment;
        let result = self
            .command_runner
            .run(command)
            .map_err(|error| RuntimeNodeError::terminal(error.to_string()))?;
        if result.exit_code != 0 {
            return Err(RuntimeNodeError::terminal(format!(
                "Unable to start execution container from image {image}: {}",
                result.stderr.trim()
            )));
        }
        self.container_id = Some(if result.stdout.trim().is_empty() {
            self.container_name.clone()
        } else {
            result.stdout.trim().to_string()
        });
        Ok(())
    }
}

impl NodeExecutor for ContainerizedNodeExecutor {
    fn execute(&mut self, request: NodeExecutionRequest) -> Result<Outcome, RuntimeNodeError> {
        match self.selection.profile.mode {
            ExecutionMode::Native => self.inner.execute(request),
            ExecutionMode::LocalContainer => self.execute_container(request),
        }
    }

    fn finalize(&mut self) {
        let _ = self.close();
    }

    fn take_cleanup_error(&mut self) -> Option<String> {
        self.last_cleanup_error.take()
    }
}

impl Drop for ContainerizedNodeExecutor {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn format_diagnostic(message: &str, result: &CommandResult) -> String {
    if result.stderr.trim().is_empty() {
        format!("{message} (exit code {})", result.exit_code)
    } else {
        format!(
            "{message} (exit code {}): {}",
            result.exit_code,
            result.stderr.trim()
        )
    }
}

fn container_env(
    context: &attractor_core::ContextMap,
    flow: &Value,
    env: &impl spark_common::paths::Environment,
) -> BTreeMap<String, String> {
    const PROVIDER_ENV_ALLOWLIST: &[&str] = &[
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "OPENAI_PROJECT_ID",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "GEMINI_API_KEY",
        "GEMINI_BASE_URL",
        "GOOGLE_API_KEY",
        "OPENROUTER_API_KEY",
        "OPENROUTER_BASE_URL",
        "OPENROUTER_HTTP_REFERER",
        "OPENROUTER_TITLE",
        "LITELLM_BASE_URL",
        "LITELLM_API_KEY",
        "OPENAI_COMPATIBLE_BASE_URL",
        "OPENAI_COMPATIBLE_API_KEY",
    ];
    let mut keys: std::collections::BTreeSet<String> = PROVIDER_ENV_ALLOWLIST
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    if let Some(captured) = context.get("internal.execution_configuration_snapshot") {
        let mut selections = std::collections::BTreeSet::new();
        fn inspect(value: &Value, selections: &mut std::collections::BTreeSet<String>) {
            match value {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        if [
                            "provider",
                            "llm_provider",
                            "llm_profile",
                            "_attractor.runtime.launch_profile",
                            "_attractor.runtime.launch_provider",
                        ]
                        .contains(&key.as_str())
                        {
                            if let Some(value) = value.as_str() {
                                selections.insert(value.trim().to_owned());
                            }
                        } else if !key.starts_with("internal.") {
                            inspect(value, selections);
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        inspect(value, selections);
                    }
                }
                _ => (),
            }
        }
        inspect(&serde_json::json!(context), &mut selections);
        inspect(flow, &mut selections);
        // Only credential references for selected providers/profiles cross the boundary.
        for section in [
            captured.get("providers"),
            context.get("internal.llm_profiles_snapshot"),
        ] {
            if let Some(profiles) = section.and_then(Value::as_object) {
                for (id, profile) in profiles {
                    if !selections.contains(id) {
                        continue;
                    }
                    if let Some(key) = profile.get("api_key_env").and_then(Value::as_str) {
                        if !key.is_empty()
                            && key.bytes().enumerate().all(|(i, c)| {
                                c == b'_'
                                    || c.is_ascii_alphabetic()
                                    || (i > 0 && c.is_ascii_digit())
                            })
                        {
                            keys.insert(key.to_owned());
                        }
                    }
                }
            }
        }
    }
    keys.into_iter()
        .filter_map(|key| env.get_var(&key).map(|value| (key, value)))
        .collect()
}

/// Extra bind mounts declared by the execution profile as
/// `metadata."container.mounts" = ["host:container[:options]", ...]`,
/// letting profiles provide runtime resources (for example agent
/// credentials, mounted read-only) without baking them into image layers.
pub(crate) fn profile_mounts(
    profile: &crate::profile::ExecutionProfile,
) -> Result<Vec<String>, RuntimeNodeError> {
    let Some(declared) = profile.metadata.get("container.mounts") else {
        return Ok(Vec::new());
    };
    let Some(entries) = declared.as_array() else {
        return Err(RuntimeNodeError::terminal(
            "container.mounts profile metadata must be an array of strings",
        ));
    };
    let mut mounts = Vec::new();
    for entry in entries {
        let Some(spec) = entry
            .as_str()
            .map(str::trim)
            .filter(|spec| !spec.is_empty())
        else {
            return Err(RuntimeNodeError::terminal(
                "container.mounts entries must be non-empty strings",
            ));
        };
        let parts: Vec<&str> = spec.split(':').collect();
        if !(2..=3).contains(&parts.len()) || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(RuntimeNodeError::terminal(format!(
                "container.mounts entry {spec:?} must be host:container or host:container:options",
            )));
        }
        mounts.push(spec.to_string());
    }
    Ok(mounts)
}

/// Test-only re-export of the profile mount parser.
pub fn profile_mounts_for_test(
    profile: &crate::profile::ExecutionProfile,
) -> Result<Vec<String>, RuntimeNodeError> {
    profile_mounts(profile)
}

fn mount_arg(source: &Path, target: &Path) -> String {
    format!("{}:{}:rw", host_path(source).display(), target.display())
}

fn host_path(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for (container_prefix, env_key) in [
        (Path::new("/projects"), "SPARK_PROJECTS_HOST_DIR"),
        (Path::new("/spark"), "SPARK_DOCKER_HOME"),
    ] {
        if let Ok(relative) = resolved.strip_prefix(container_prefix) {
            if let Ok(host_prefix) = std::env::var(env_key) {
                return PathBuf::from(host_prefix).join(relative);
            }
        }
    }
    resolved
}

fn dedupe(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeMap::<String, ()>::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone(), ()).is_none())
        .collect()
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn _failure_outcome(message: impl Into<String>) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: message.into(),
        retryable: Some(false),
        failure_kind: Some(FailureKind::Runtime),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_selected_credential_references_cross_the_environment_boundary() {
        let context = BTreeMap::from([
            ("_attractor.runtime.launch_profile".into(), json!("team")),
            (
                "internal.llm_profiles_snapshot".into(),
                json!({"team":{"api_key_env":"TEAM_KEY"}, "unused":{"api_key_env":"UNUSED_KEY"}}),
            ),
            (
                "internal.execution_configuration_snapshot".into(),
                json!({"providers":{"openai":{"api_key_env":"CUSTOM_OPENAI_KEY"}}}),
            ),
        ]);
        let env = BTreeMap::from([
            ("TEAM_KEY".into(), "secret-test-sentinel".into()),
            ("UNUSED_KEY".into(), "unused".into()),
            ("CUSTOM_OPENAI_KEY".into(), "provider-test-sentinel".into()),
            ("HOME".into(), "/host-home".into()),
            ("CODEX_HOME".into(), "/host-codex".into()),
        ]);
        let result = container_env(
            &context,
            &json!({"nodes":[{"llm_provider":"openai"}]}),
            &env,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result["TEAM_KEY"], "secret-test-sentinel");
        assert!(!serde_json::to_string(&context)
            .unwrap()
            .contains("secret-test-sentinel"));
        let mut spec = CommandSpec::new("docker", ["run", "-e", "TEAM_KEY"]);
        spec.env = result;
        assert!(!format!("{spec:?}").contains("secret-test-sentinel"));
    }

    struct TargetShell {
        home: PathBuf,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl ContainerCommandRunner for TargetShell {
        fn command_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&mut self, spec: CommandSpec) -> std::io::Result<CommandResult> {
            if spec.args.first().is_some_and(|arg| arg == "rm") {
                return Ok(CommandResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(&spec.args[..4], ["exec", "test-container", "sh", "-c"]);
            let output = Command::new("/bin/sh")
                .args(&spec.args[3..])
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", self.home.join("bin"))
                .current_dir(&self.home)
                .output()?;
            Ok(CommandResult {
                exit_code: output.status.code().unwrap_or(1),
                stdout: String::from_utf8(output.stdout).unwrap(),
                stderr: String::from_utf8(output.stderr).unwrap(),
            })
        }
    }

    #[test]
    fn paths_resolve_in_target_home_and_path_and_are_frozen_for_active_work() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("bin")).unwrap();
        let binary = home.path().join("bin/codex");
        std::fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut executor = ContainerizedNodeExecutor::new(
            ExecutionProfileSelection {
                profile: crate::profile::ExecutionProfile::implementation_native(),
                selected_profile_id: "test".into(),
                selection_source: "test".into(),
            },
            RuntimeHandlerRunner::new(),
        )
        .with_command_runner(TargetShell {
            home: home.path().into(),
            calls: calls.clone(),
        });
        executor.container_id = Some("test-container".into());
        let mut context = BTreeMap::from([(
            "internal.execution_configuration_snapshot".into(),
            json!({"agents":{"native":{"claude_config_dir":"~/custom-claude"}}}),
        )]);
        executor.resolve_target_paths(&mut context).unwrap();
        let native = &context["internal.execution_configuration_snapshot"]["agents"]["native"];
        assert_eq!(native["codex_binary"], binary.to_string_lossy().as_ref());
        assert_eq!(
            native["codex_runtime_root"],
            home.path()
                .join(".spark/runtime/codex")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            native["claude_config_dir"],
            home.path().join("custom-claude").to_string_lossy().as_ref()
        );
        let captured = native.clone();
        executor.resolve_target_paths(&mut context).unwrap();
        assert_eq!(
            context["internal.execution_configuration_snapshot"]["agents"]["native"],
            captured
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
