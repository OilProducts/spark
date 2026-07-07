use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use attractor_core::{FailureKind, Outcome, OutcomeStatus, RawRuntimeEvent};
use attractor_runtime::{
    append_event, NodeExecutionRequest, NodeExecutor, RuntimeHandlerRunner, RuntimeNodeError,
};
use serde_json::Value;

use crate::modes::ExecutionMode;
use crate::profile::ExecutionProfileSelection;
use crate::protocol::{outcome_from_payload, WorkerFrame, WorkerNodeRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: String,
    pub env: BTreeMap<String, String>,
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
}

pub struct ContainerizedNodeExecutor {
    inner: RuntimeHandlerRunner,
    selection: ExecutionProfileSelection,
    command_runner: Box<dyn ContainerCommandRunner>,
    container_id: Option<String>,
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
            .map_err(|error| error.to_string())?;
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
        let worker_request = WorkerNodeRequest {
            run_id: request.run_id.clone(),
            flow: request.flow.clone(),
            node_id: request.node_id.clone(),
            prompt: request.prompt.clone(),
            context: request.context.clone(),
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
        let result = self.command_runner.run(spec).map_err(|error| {
            RuntimeNodeError::terminal(format!("Container node worker failed: {error}"))
        })?;
        let parsed = parse_worker_output(&result.stdout, &request)?;
        if result.exit_code != 0 {
            return Err(RuntimeNodeError::terminal(format!(
                "Container node worker failed with exit code {}: {}",
                result.exit_code,
                result.stderr.trim()
            )));
        }
        let Some(mut outcome) = parsed else {
            return Err(RuntimeNodeError::terminal(
                "Container node worker exited without a result payload.",
            ));
        };
        if self.cleanup_after_execute {
            let _ = self.close();
        }
        for (key, value) in request.context {
            outcome.context_updates.entry(key).or_insert(value);
        }
        Ok(outcome)
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
        for mount in dedupe([
            mount_arg(&request.run_workdir, &request.run_workdir),
            mount_arg(
                run_root.parent().unwrap_or(run_root.as_path()),
                run_root.parent().unwrap_or(run_root.as_path()),
            ),
            mount_arg(&run_root, &run_root),
        ]) {
            args.push("-v".to_string());
            args.push(mount);
        }
        for (key, value) in container_env() {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }
        args.extend([
            image.to_string(),
            "tail".to_string(),
            "-f".to_string(),
            "/dev/null".to_string(),
        ]);
        let result = self
            .command_runner
            .run(CommandSpec::new(&self.docker_program, args))
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

    fn take_cleanup_error(&mut self) -> Option<String> {
        self.last_cleanup_error.take()
    }
}

impl Drop for ContainerizedNodeExecutor {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn parse_worker_output(
    stdout: &str,
    request: &NodeExecutionRequest,
) -> Result<Option<Outcome>, RuntimeNodeError> {
    let mut result = None;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let frame = serde_json::from_str::<WorkerFrame>(line).map_err(|error| {
            RuntimeNodeError::terminal(format!("invalid worker protocol frame: {error}"))
        })?;
        match frame {
            WorkerFrame::Result(frame) => {
                let mut outcome = outcome_from_payload(&frame.outcome);
                for (key, value) in frame.context {
                    outcome.context_updates.insert(key, value);
                }
                result = Some(outcome);
            }
            WorkerFrame::Event(frame) => {
                if let Some(paths) = &request.run_paths {
                    let mut event = RawRuntimeEvent::new(frame.event_type, request.run_id.clone());
                    event.payload = frame.payload;
                    append_event(paths, event)
                        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

fn container_env() -> BTreeMap<String, String> {
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
        "HOME",
        "CODEX_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        "SPARK_HOME",
    ];
    PROVIDER_ENV_ALLOWLIST
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
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
