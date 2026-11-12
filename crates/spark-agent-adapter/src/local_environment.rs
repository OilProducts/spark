use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use glob::Pattern;
use regex::RegexBuilder;

use crate::environment::{
    CommandOptions, DirEntry, EnvironmentError, EnvironmentInheritancePolicy, EnvironmentResult,
    ExecResult, ExecutionEnvironment, ExecutionEnvironmentBackend, GrepOptions,
};

const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 10_000;
const MAX_COMMAND_TIMEOUT_MS: u64 = 600_000;
const TIMEOUT_EXIT_CODE: i32 = 124;
const TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct LocalExecutionEnvironment {
    configured_working_directory: PathBuf,
    resolved_working_directory: PathBuf,
    default_command_timeout_ms: u64,
    max_command_timeout_ms: u64,
    environment_inheritance_policy: EnvironmentInheritancePolicy,
    active_process_groups: Arc<Mutex<HashSet<u32>>>,
}

impl Default for LocalExecutionEnvironment {
    fn default() -> Self {
        Self::new(env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl LocalExecutionEnvironment {
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self::with_options(
            working_dir,
            DEFAULT_COMMAND_TIMEOUT_MS,
            MAX_COMMAND_TIMEOUT_MS,
            EnvironmentInheritancePolicy::InheritCoreOnly,
        )
    }

    pub fn with_options(
        working_dir: impl Into<PathBuf>,
        default_command_timeout_ms: u64,
        max_command_timeout_ms: u64,
        environment_inheritance_policy: EnvironmentInheritancePolicy,
    ) -> Self {
        let configured_working_directory = working_dir.into();
        let resolved_working_directory = absolutize(&configured_working_directory);
        Self {
            configured_working_directory,
            resolved_working_directory,
            default_command_timeout_ms,
            max_command_timeout_ms,
            environment_inheritance_policy,
            active_process_groups: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn into_execution_environment(self) -> ExecutionEnvironment {
        let working_dir = Some(self.configured_working_directory.clone());
        ExecutionEnvironment::from_backend_arc(Arc::new(self), working_dir)
    }

    pub fn with_working_directory(&self, working_dir: impl Into<PathBuf>) -> Self {
        Self::with_options(
            working_dir,
            self.default_command_timeout_ms,
            self.max_command_timeout_ms,
            self.environment_inheritance_policy,
        )
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.resolved_working_directory.join(path)
        };
        normalize_path(candidate)
    }

    fn resolve_command_cwd(&self, working_dir: Option<&Path>) -> EnvironmentResult<PathBuf> {
        let cwd = working_dir
            .map(|path| self.resolve_path(path))
            .unwrap_or_else(|| self.resolved_working_directory.clone());
        if !cwd.exists() {
            return Err(EnvironmentError::FileNotFound(cwd));
        }
        if !cwd.is_dir() {
            return Err(EnvironmentError::NotDirectory(cwd));
        }
        Ok(cwd)
    }

    fn display_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.resolved_working_directory)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn build_environment(&self, env_vars: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut environment = match self.environment_inheritance_policy {
            EnvironmentInheritancePolicy::InheritAll => env::vars().collect(),
            EnvironmentInheritancePolicy::InheritNone => BTreeMap::new(),
            EnvironmentInheritancePolicy::InheritCoreOnly => env::vars()
                .filter(|(key, _)| is_core_env_key(key) && !is_sensitive_env_key(key))
                .collect(),
        };
        environment.extend(env_vars.clone());
        environment
    }

    fn register_process_group(&self, process_group_id: u32) {
        self.active_process_groups
            .lock()
            .expect("active process group lock")
            .insert(process_group_id);
    }

    fn unregister_process_group(&self, process_group_id: u32) {
        self.active_process_groups
            .lock()
            .expect("active process group lock")
            .remove(&process_group_id);
    }

    fn terminate_process_group(&self, process_group_id: u32, child: Option<&mut Child>) {
        terminate_process_group(process_group_id, child);
    }

    fn grep_with_ripgrep(
        &self,
        rg_path: PathBuf,
        search_target: &Path,
        pattern: &str,
        options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        let mut command = Command::new(rg_path);
        command.arg("--json").arg("--no-config");
        if options.case_insensitive {
            command.arg("-i");
        }
        if let Some(glob_filter) = options.glob_filter.as_ref() {
            command.arg("-g").arg(glob_filter);
        }
        command
            .arg("-e")
            .arg(pattern)
            .arg(search_target)
            .current_dir(&self.resolved_working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = command
            .output()
            .map_err(|error| EnvironmentError::CommandStart(error.to_string()))?;
        if output.status.code() == Some(1) {
            return Ok(String::new());
        }
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            };
            return Err(EnvironmentError::InvalidInput(message));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut matches = Vec::new();
        for line in stdout.lines() {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if record.get("type").and_then(|value| value.as_str()) != Some("match") {
                continue;
            }
            let Some(data) = record.get("data") else {
                continue;
            };
            let Some(path_text) = data
                .get("path")
                .and_then(|value| value.get("text"))
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let Some(line_text) = data
                .get("lines")
                .and_then(|value| value.get("text"))
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let Some(line_number) = data.get("line_number").and_then(|value| value.as_u64()) else {
                continue;
            };
            let display_path = self.display_path(&absolutize(Path::new(path_text)));
            matches.push(format!(
                "{}:{}:{}",
                display_path,
                line_number,
                line_text.trim_end_matches(['\r', '\n'])
            ));
            if matches.len() >= options.max_results {
                break;
            }
        }
        Ok(matches.join("\n"))
    }

    fn grep_with_regex(
        &self,
        search_target: &Path,
        pattern: &str,
        options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(options.case_insensitive)
            .build()
            .map_err(|error| EnvironmentError::InvalidInput(error.to_string()))?;
        let glob_filter = options
            .glob_filter
            .as_deref()
            .map(Pattern::new)
            .transpose()
            .map_err(|error| EnvironmentError::InvalidInput(error.to_string()))?;

        let mut matches = Vec::new();
        for file_path in grep_files(search_target) {
            let relative_path = self.display_path(&file_path);
            if let Some(glob_filter) = glob_filter.as_ref() {
                if !glob_filter.matches(&relative_path) {
                    continue;
                }
            }
            let content = fs::read_to_string(&file_path)
                .map_err(|error| map_io_error(error, file_path.clone()))?;
            for (line_index, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(format!("{}:{}:{}", relative_path, line_index + 1, line));
                    if matches.len() >= options.max_results {
                        return Ok(matches.join("\n"));
                    }
                }
            }
        }
        Ok(matches.join("\n"))
    }
}

impl std::fmt::Debug for LocalExecutionEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalExecutionEnvironment")
            .field(
                "configured_working_directory",
                &self.configured_working_directory,
            )
            .field(
                "default_command_timeout_ms",
                &self.default_command_timeout_ms,
            )
            .field("max_command_timeout_ms", &self.max_command_timeout_ms)
            .field(
                "environment_inheritance_policy",
                &self.environment_inheritance_policy,
            )
            .finish()
    }
}

impl ExecutionEnvironmentBackend for LocalExecutionEnvironment {
    fn read_file(
        &self,
        path: &Path,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        let target = self.resolve_path(path);
        if !target.exists() {
            return Err(EnvironmentError::FileNotFound(target));
        }
        if target.is_dir() {
            return Err(EnvironmentError::IsDirectory(target));
        }
        if matches!(offset, Some(0)) {
            return Err(EnvironmentError::InvalidInput(
                "offset must be at least 1".to_string(),
            ));
        }
        let raw = fs::read(&target).map_err(|error| map_io_error(error, target.clone()))?;
        let content = String::from_utf8(raw).map_err(|_| EnvironmentError::InvalidUtf8(target))?;
        if offset.is_none() && limit.is_none() {
            return Ok(content);
        }
        let start = offset.unwrap_or(1).saturating_sub(1);
        let end = limit.map(|limit| start.saturating_add(limit));
        let lines = content
            .split_inclusive('\n')
            .enumerate()
            .filter_map(|(index, line)| {
                if index < start {
                    return None;
                }
                if end.is_some_and(|end| index >= end) {
                    return None;
                }
                Some(line)
            })
            .collect::<String>();
        Ok(lines)
    }

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>> {
        let target = self.resolve_path(path);
        if !target.exists() {
            return Err(EnvironmentError::FileNotFound(target));
        }
        if target.is_dir() {
            return Err(EnvironmentError::IsDirectory(target));
        }
        fs::read(&target).map_err(|error| map_io_error(error, target))
    }

    fn write_file(&self, path: &Path, content: &str) -> EnvironmentResult<()> {
        let target = self.resolve_path(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| map_io_error(error, parent.to_path_buf()))?;
        }
        fs::write(&target, content).map_err(|error| map_io_error(error, target))
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.resolve_path(path).exists()
    }

    fn is_directory(&self, path: &Path) -> bool {
        self.resolve_path(path).is_dir()
    }

    fn delete_file(&self, path: &Path) -> EnvironmentResult<()> {
        let target = self.resolve_path(path);
        if !target.exists() {
            return Err(EnvironmentError::FileNotFound(target));
        }
        if target.is_dir() {
            return Err(EnvironmentError::IsDirectory(target));
        }
        fs::remove_file(&target).map_err(|error| map_io_error(error, target))
    }

    fn rename_file(&self, source_path: &Path, destination_path: &Path) -> EnvironmentResult<()> {
        let source = self.resolve_path(source_path);
        let destination = self.resolve_path(destination_path);
        if !source.exists() {
            return Err(EnvironmentError::FileNotFound(source));
        }
        if source.is_dir() {
            return Err(EnvironmentError::IsDirectory(source));
        }
        if destination.exists() {
            return Err(EnvironmentError::AlreadyExists(destination));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| map_io_error(error, parent.to_path_buf()))?;
        }
        fs::rename(&source, &destination).map_err(|error| map_io_error(error, source))
    }

    fn list_directory(&self, path: &Path, depth: usize) -> EnvironmentResult<Vec<DirEntry>> {
        let base = self.resolve_path(path);
        if !base.exists() {
            return Err(EnvironmentError::FileNotFound(base));
        }
        if !base.is_dir() {
            return Err(EnvironmentError::NotDirectory(base));
        }

        let mut entries = Vec::new();
        collect_directory_entries(&base, depth, Path::new(""), &mut entries)?;
        Ok(entries)
    }

    fn exec_command(
        &self,
        command: &str,
        options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        let timeout_ms = options
            .timeout_ms
            .unwrap_or(self.default_command_timeout_ms)
            .min(self.max_command_timeout_ms);
        if timeout_ms < 1 {
            return Err(EnvironmentError::InvalidInput(
                "timeout_ms must be at least 1".to_string(),
            ));
        }
        let cwd = self.resolve_command_cwd(options.working_dir.as_deref())?;
        let environment = self.build_environment(&options.env_vars);
        let mut command_builder = shell_command(command);
        command_builder
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .envs(environment);
        configure_process_group(&mut command_builder);

        let aborted = || {
            options
                .abort_signal
                .as_ref()
                .is_some_and(unified_llm_adapter::AbortSignal::aborted)
        };
        if aborted() {
            return Err(EnvironmentError::Other("Stopped.".to_string()));
        }
        let start = Instant::now();
        let mut child = spawn_process_group(command_builder)
            .map_err(|error| EnvironmentError::CommandStart(error.to_string()))?;
        let process_group_id = child.id();
        self.register_process_group(process_group_id);

        let stdout_handle = child.stdout.take().map(read_pipe);
        let stderr_handle = child.stderr.take().map(read_pipe);
        let deadline = start + Duration::from_millis(timeout_ms);
        let mut timed_out = false;
        let mut stopped = false;
        let mut exit_status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| EnvironmentError::Other(error.to_string()))?
            {
                break Some(status);
            }
            if Instant::now() >= deadline {
                timed_out = true;
                self.terminate_process_group(process_group_id, Some(&mut child));
                break child.try_wait().ok().flatten();
            }
            // Checked after registration, so an abort whose cleanup ran
            // before this command registered still kills it.
            if aborted() {
                stopped = true;
                self.terminate_process_group(process_group_id, Some(&mut child));
                break child.try_wait().ok().flatten();
            }
            thread::sleep(Duration::from_millis(5));
        };

        if exit_status.is_none() {
            exit_status = child.wait().ok();
        }
        self.unregister_process_group(process_group_id);

        let stdout = join_pipe(stdout_handle)?;
        let mut stderr = join_pipe(stderr_handle)?;
        if timed_out {
            stderr = append_timeout_message(stderr, &timeout_message(timeout_ms));
        }
        if stopped {
            stderr = append_timeout_message(stderr, "Stopped.");
        }
        let duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let exit_code = if timed_out {
            TIMEOUT_EXIT_CODE
        } else {
            exit_status.and_then(|status| status.code()).unwrap_or(1)
        };

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
            timed_out,
            duration_ms,
        })
    }

    fn grep(&self, pattern: &str, path: &Path, options: &GrepOptions) -> EnvironmentResult<String> {
        options.validate()?;
        let search_target = self.resolve_path(path);
        if !search_target.exists() {
            return Err(EnvironmentError::FileNotFound(search_target));
        }
        if let Some(rg_path) = find_executable("rg") {
            return self.grep_with_ripgrep(rg_path, &search_target, pattern, options);
        }
        self.grep_with_regex(&search_target, pattern, options)
    }

    fn glob(&self, pattern: &str, path: &Path) -> EnvironmentResult<Vec<String>> {
        if Path::new(pattern).is_absolute() {
            return Err(EnvironmentError::InvalidInput(
                "absolute glob patterns are not supported".to_string(),
            ));
        }
        let base = self.resolve_path(path);
        if !base.exists() {
            return Err(EnvironmentError::FileNotFound(base));
        }
        if !base.is_dir() {
            return Err(EnvironmentError::NotDirectory(base));
        }
        let full_pattern = base.join(pattern).to_string_lossy().to_string();
        let mut matches = Vec::new();
        for candidate in glob::glob(&full_pattern)
            .map_err(|error| EnvironmentError::InvalidInput(error.to_string()))?
        {
            let candidate =
                candidate.map_err(|error| EnvironmentError::InvalidInput(error.to_string()))?;
            if candidate.is_file() {
                let modified = candidate
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                matches.push((modified, self.display_path(&candidate), candidate));
            }
        }
        matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        Ok(matches.into_iter().map(|(_, path, _)| path).collect())
    }

    fn initialize(&self) -> EnvironmentResult<()> {
        fs::create_dir_all(&self.resolved_working_directory)
            .map_err(|error| map_io_error(error, self.resolved_working_directory.clone()))
    }

    fn cleanup(&self) -> EnvironmentResult<()> {
        let process_groups = self
            .active_process_groups
            .lock()
            .expect("active process group lock")
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for process_group_id in process_groups {
            self.terminate_process_group(process_group_id, None);
            self.unregister_process_group(process_group_id);
        }
        Ok(())
    }

    fn working_directory(&self) -> String {
        self.configured_working_directory
            .to_string_lossy()
            .to_string()
    }

    fn platform(&self) -> String {
        platform_name().to_string()
    }

    fn os_version(&self) -> String {
        os_version()
    }
}

fn collect_directory_entries(
    current: &Path,
    remaining_depth: usize,
    relative_prefix: &Path,
    entries: &mut Vec<DirEntry>,
) -> EnvironmentResult<()> {
    let mut children = fs::read_dir(current)
        .map_err(|error| map_io_error(error, current.to_path_buf()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_io_error(error, current.to_path_buf()))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let child_path = child.path();
        let child_name = relative_prefix.join(child.file_name());
        let metadata = child
            .metadata()
            .map_err(|error| map_io_error(error, child_path.clone()))?;
        let is_dir = metadata.is_dir();
        entries.push(DirEntry {
            name: child_name.to_string_lossy().replace('\\', "/"),
            is_dir,
            size: if is_dir { None } else { Some(metadata.len()) },
        });
        if is_dir && remaining_depth > 0 {
            collect_directory_entries(&child_path, remaining_depth - 1, &child_name, entries)?;
        }
    }
    Ok(())
}

fn grep_files(search_target: &Path) -> Vec<PathBuf> {
    if search_target.is_file() {
        return vec![search_target.to_path_buf()];
    }
    let mut files = Vec::new();
    collect_files(search_target, &mut files);
    files.sort();
    files
}

fn collect_files(current: &Path, files: &mut Vec<PathBuf>) {
    let Ok(children) = fs::read_dir(current) else {
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn read_pipe<R>(mut pipe: R) -> thread::JoinHandle<std::io::Result<String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = String::new();
        pipe.read_to_string(&mut output)?;
        Ok(output)
    })
}

fn join_pipe(
    handle: Option<thread::JoinHandle<std::io::Result<String>>>,
) -> EnvironmentResult<String> {
    let Some(handle) = handle else {
        return Ok(String::new());
    };
    handle
        .join()
        .map_err(|_| EnvironmentError::Other("output reader thread panicked".to_string()))?
        .map_err(|error| EnvironmentError::Other(error.to_string()))
}

fn append_timeout_message(mut output: String, message: &str) -> String {
    if output.is_empty() {
        return message.to_string();
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(message);
    output
}

fn timeout_message(timeout_ms: u64) -> String {
    format!(
        "[ERROR: Command timed out after {timeout_ms}ms. Partial output is shown above.\n\
You can retry with a longer timeout by setting the timeout_ms parameter.]"
    )
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let shell = env::var_os("COMSPEC").unwrap_or_else(|| std::ffi::OsString::from("cmd.exe"));
        let mut command_builder = Command::new(shell);
        command_builder.arg("/C").arg(command);
        command_builder
    }

    #[cfg(not(windows))]
    {
        let mut command_builder = Command::new("/bin/bash");
        command_builder.arg("-c").arg(command);
        command_builder
    }
}

/// Makes `command` lead its own process group, which `terminate_process_group`
/// stops as a whole. Start it with `spawn_process_group`.
pub fn configure_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    #[cfg(windows)]
    {
        let _ = command;
    }
}

/// Starts `command`, set up by `configure_process_group`, tied to this process:
/// once this process ends, however it ends, the command and everything it
/// started stop within a few seconds. Until then nothing changes. If it cannot
/// be tied, it does not start.
#[allow(unsafe_code)]
pub fn spawn_process_group(mut command: Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let lifeline = match LIFELINE.get() {
            Some(lifeline) => lifeline,
            None => {
                let pipe = std::io::pipe()?;
                LIFELINE.get_or_init(move || pipe)
            }
        };
        let input = lifeline.0.try_clone()?;
        let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
        theirs.set_read_timeout(Some(Duration::from_millis(100)))?;
        let spark = rustix::process::getpid();
        // SAFETY: between fork and exec it only makes system calls.
        unsafe {
            command.pre_exec(move || await_lifeline(&theirs, spark));
        }
        thread::Builder::new()
            .name("spark-lifeline".to_string())
            .spawn(move || guard_group(ours, input))?;
        command.spawn()
    }

    #[cfg(windows)]
    {
        join_spark_job()?;
        command.spawn()
    }
}

/// A pipe whose close-on-exec write end only this process holds, so its read
/// end sees end of input once this process has ended, however it ended.
#[cfg(unix)]
static LIFELINE: std::sync::OnceLock<(std::io::PipeReader, std::io::PipeWriter)> =
    std::sync::OnceLock::new();

/// Stops its process group, `$1`, once Spark's pipe on its stdin reaches end of
/// input: TERM, then KILL after a grace period. It outlasts a TERM to the
/// group, Spark's own stop, still watching the pipe meanwhile, and goes once
/// nothing is left of the group but itself and `$2`, a leader that goes by
/// itself. Its first reader says when it is ready, once a TERM would end it.
#[cfg(unix)]
const LIFELINE_SCRIPT: &str = "trap 'stop=1' TERM; exec 3<&0; group=$1 skip=$2; \
    others() { ps -A -o pgid=,ppid=,pid=,stat= | awk -v g=\"$group\" -v me=$$ -v skip=\"$skip\" \
    '$1 == g && $2 != me && $3 != me && $3 != skip && $4 !~ /^Z/ { f = 1 } END { exit !f }'; }; \
    ( echo; exec >/dev/null; read _ <&3 ) & exec >/dev/null; wait $!; \
    while [ \"$stop\" ]; do \
    stop=; ( read _ <&3 ) & reader=$!; n=0; \
    while [ $n -lt 40 ] && kill -0 $reader 2>/dev/null; do \
    others || { kill -KILL $reader; exit 0; }; sleep 0.05; n=$((n + 1)); done; \
    wait $reader; done; \
    kill -TERM 0; sleep 2; kill -KILL 0";

/// Runs in the started process between fork and exec, so its program runs
/// only once its process group has a lifeline: it tells `guard_group` its
/// group and itself and waits for the answer, giving up if Spark is gone
/// meanwhile.
#[cfg(unix)]
fn await_lifeline(
    socket: &std::os::unix::net::UnixStream,
    spark: rustix::process::Pid,
) -> std::io::Result<()> {
    use rustix::io::{read, write, Errno};
    use rustix::process::{getpgrp, getpid, getppid};

    let mut ids = [0; 8];
    ids[..4].copy_from_slice(&getpgrp().as_raw_pid().to_ne_bytes());
    ids[4..].copy_from_slice(&getpid().as_raw_pid().to_ne_bytes());
    write(socket, &ids)?;
    let mut answer = [0; 4];
    loop {
        match read(socket, &mut answer) {
            Ok(4) if answer == [0; 4] => return Ok(()),
            Ok(4) => {
                return Err(std::io::Error::from_raw_os_error(i32::from_ne_bytes(
                    answer,
                )))
            }
            Ok(_) => return Err(Errno::CANCELED.into()),
            Err(Errno::AGAIN | Errno::INTR) if getppid() == Some(spark) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// Puts a lifeline, reading Spark's pipe, into the group of the process
/// `await_lifeline` holds back, and answers it with the outcome. Being in the
/// group keeps the group's ID from being reused, so it never signals anything
/// Spark did not start. Once nothing else is left in the group, the group goes.
#[cfg(unix)]
fn guard_group(mut socket: std::os::unix::net::UnixStream, input: std::io::PipeReader) {
    use std::io::Write;

    let mut ids = [0; 8];
    if socket.read_exact(&mut ids).is_err() {
        return; // It never got that far.
    }
    let [group, pid] =
        [0, 4].map(|at| i32::from_ne_bytes([ids[at], ids[at + 1], ids[at + 2], ids[at + 3]]));
    let lifeline = if group == rustix::process::getpgrp().as_raw_pid() {
        Err(std::io::Error::other("it shares Spark's process group"))
    } else {
        // A leader it joined goes by itself once the rest of the group has.
        start_lifeline(group, if group == pid { 0 } else { group }, input)
    };
    let answer = match &lifeline {
        Ok(_) => 0,
        Err(error) => error
            .raw_os_error()
            .unwrap_or(rustix::io::Errno::CANCELED.raw_os_error()),
    };
    let _ = socket.write_all(&i32::to_ne_bytes(answer));
    drop(socket);
    let (Ok(lifeline), Some(leader)) = (lifeline, rustix::process::Pid::from_raw(group)) else {
        return;
    };
    let mut guarded = GUARDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guarded.push((leader, lifeline, 0));
    // The first to arrive while no group is guarded watches them all, until
    // none are left.
    if guarded.len() == 1 {
        drop(guarded);
        retire_lifelines();
    }
}

/// The lifelines of groups that may still have more in them: the group's
/// leader, its lifeline, and how many times in a row it was found alone.
#[cfg(unix)]
static GUARDED: Mutex<Vec<(rustix::process::Pid, Child, u8)>> = Mutex::new(Vec::new());

/// Once a second, ends each guarded group that nothing but its lifeline is left
/// in, with one `ps` for them all, until no group is left to guard.
#[cfg(unix)]
fn retire_lifelines() {
    use rustix::process::{kill_process_group, test_kill_process, Signal};

    loop {
        thread::sleep(Duration::from_secs(1));
        let mut guarded = GUARDED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A lifeline goes by itself after a stop.
        guarded.retain_mut(|(_, lifeline, _)| matches!(lifeline.try_wait(), Ok(None)));
        let mut listing = None;
        for (leader, lifeline, alone) in guarded.iter_mut() {
            let others = test_kill_process(*leader).is_ok()
                || has_other_members(
                    listing.get_or_insert_with(list_processes),
                    leader.as_raw_pid(),
                    lifeline.id(),
                );
            *alone = if others { 0 } else { *alone + 1 };
        }
        // Twice in a row, since `ps` takes no snapshot.
        guarded.retain_mut(|(leader, lifeline, alone)| {
            if *alone < 2 {
                return true;
            }
            // Not yet reaped, it still holds the group's ID.
            let _ = kill_process_group(*leader, Signal::KILL);
            let _ = lifeline.wait();
            false
        });
        if guarded.is_empty() {
            return;
        }
    }
}

#[cfg(unix)]
fn start_lifeline(group: i32, skip: i32, input: std::io::PipeReader) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;

    let mut lifeline = Command::new("sh")
        .args(["-c", LIFELINE_SCRIPT, "lifeline"])
        .args([group.to_string(), skip.to_string()])
        .process_group(group)
        .current_dir("/")
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let ready = lifeline
        .stdout
        .take()
        .map(|mut ready| ready.read_exact(&mut [0]));
    if let Some(Ok(())) = ready {
        return Ok(lifeline);
    }
    let _ = lifeline.kill();
    let _ = lifeline.wait();
    Err(std::io::Error::other("Spark's lifeline did not start"))
}

/// Every process, as `ps` lists it, if it can.
#[cfg(unix)]
fn list_processes() -> Option<String> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,pgid=,stat="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether a live process other than `lifeline` and its own is in `group`, or
/// `listing` cannot tell.
#[cfg(unix)]
fn has_other_members(listing: &Option<String>, group: i32, lifeline: u32) -> bool {
    let Some(listing) = listing else {
        return true;
    };
    let (group, lifeline) = (group.to_string(), lifeline.to_string());
    listing.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        matches!(fields[..], [pid, parent, pgid, stat, ..]
            if pid != lifeline && parent != lifeline && pgid == group && !stat.starts_with('Z'))
    })
}

/// Puts this process, once, into a job that ends every process in it when its
/// handle closes. Only this process holds that handle, so that happens once
/// this process ends, however it ends. What it starts from then on begins in
/// the job, with no moment outside it.
#[cfg(windows)]
#[allow(unsafe_code)]
fn join_spark_job() -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    static JOINED: Mutex<bool> = Mutex::new(false);
    let mut joined = JOINED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *joined {
        return Ok(());
    }
    // SAFETY: Win32 calls on a job handle made here, which stays open for the
    // life of this process once this process is in the job.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let in_job = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) != 0
            && AssignProcessToJobObject(job, GetCurrentProcess()) != 0;
        if !in_job {
            let error = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(error);
        }
    }
    *joined = true;
    Ok(())
}

/// Terminates a process group started with `configure_process_group`: TERM,
/// then KILL after a grace period, so tool subprocesses die with their agent.
pub fn terminate_process_group(process_group_id: u32, mut child: Option<&mut Child>) {
    #[cfg(unix)]
    {
        use rustix::process::{kill_process_group, test_kill_process_group, Pid, Signal};

        let Some(pid) = Pid::from_raw(process_group_id as i32) else {
            return;
        };
        let _ = kill_process_group(pid, Signal::TERM);
        let deadline = Instant::now() + TERMINATION_GRACE_PERIOD;
        while Instant::now() < deadline {
            let _ = child.as_deref_mut().and_then(|child| child.try_wait().ok());
            // Where a zombie still counts as a member, the group lasts until
            // its lifeline, gone once nothing else is left, is reaped.
            GUARDED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain_mut(|(leader, lifeline, _)| {
                    *leader != pid || matches!(lifeline.try_wait(), Ok(None))
                });
            if test_kill_process_group(pid).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if test_kill_process_group(pid).is_ok() {
            let _ = kill_process_group(pid, Signal::KILL);
        }
    }

    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(process_group_id.to_string())
            .arg("/T")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + TERMINATION_GRACE_PERIOD;
        while Instant::now() < deadline {
            if let Some(child) = child.as_deref_mut() {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(process_group_id.to_string())
            .arg("/T")
            .arg("/F")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Some(child) = child.as_deref_mut() {
            let _ = child.wait();
        }
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() && is_executable(candidate) {
        return Some(candidate.to_path_buf());
    }
    let path_var = env::var_os("PATH")?;
    for directory in env::split_paths(&path_var) {
        #[cfg(windows)]
        {
            let pathext = env::var_os("PATHEXT")
                .unwrap_or_else(|| std::ffi::OsString::from(".EXE;.BAT;.CMD"));
            for extension in pathext.to_string_lossy().split(';') {
                let candidate = directory.join(format!("{name}{extension}"));
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }

        #[cfg(not(windows))]
        {
            let candidate = directory.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn absolutize(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    if expanded.is_absolute() {
        normalize_path(expanded)
    } else {
        normalize_path(
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(expanded),
        )
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let mut components = path.components();
    let Some(Component::Normal(first)) = components.next() else {
        return path.to_path_buf();
    };
    if first != "~" {
        return path.to_path_buf();
    }
    let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) else {
        return path.to_path_buf();
    };
    let mut expanded = PathBuf::from(home);
    for component in components {
        expanded.push(component.as_os_str());
    }
    expanded
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn map_io_error(error: std::io::Error, path: PathBuf) -> EnvironmentError {
    match error.kind() {
        std::io::ErrorKind::NotFound => EnvironmentError::FileNotFound(path),
        std::io::ErrorKind::PermissionDenied => EnvironmentError::PermissionDenied(path),
        std::io::ErrorKind::AlreadyExists => EnvironmentError::AlreadyExists(path),
        _ => EnvironmentError::Io {
            path,
            source: error,
        },
    }
}

fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.ends_with("_API_KEY")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_PASSWORD")
        || upper.ends_with("_CREDENTIAL")
}

fn is_core_env_key(key: &str) -> bool {
    const CORE_KEYS: &[&str] = &[
        "APPDATA",
        "CARGO_HOME",
        "CLASSPATH",
        "COMSPEC",
        "CONDA_PREFIX",
        "DYLD_LIBRARY_PATH",
        "GOBIN",
        "GOCACHE",
        "GOMODCACHE",
        "GOPATH",
        "GRADLE_HOME",
        "HOME",
        "HOMEDRIVE",
        "HOMEPATH",
        "JAVA_HOME",
        "LANG",
        "LD_LIBRARY_PATH",
        "LOCALAPPDATA",
        "LOGNAME",
        "MANPATH",
        "NVM_DIR",
        "NODE_PATH",
        "OLDPWD",
        "PATH",
        "PNPM_HOME",
        "PKG_CONFIG_PATH",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PYTHONHOME",
        "PYTHONPATH",
        "PWD",
        "RUSTUP_HOME",
        "SDKMAN_DIR",
        "SHELL",
        "TERM",
        "TMPDIR",
        "USER",
        "USERNAME",
        "UV_CACHE_DIR",
        "UV_TOOL_DIR",
        "VIRTUAL_ENV",
        "WINDIR",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "YARN_CACHE_FOLDER",
    ];
    let upper = key.to_ascii_uppercase();
    CORE_KEYS.contains(&upper.as_str())
        || upper.ends_with("_PATH")
        || upper.ends_with("_HOME")
        || upper.ends_with("_DIR")
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_family = "wasm") {
        "wasm"
    } else {
        "linux"
    }
}

fn os_version() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(version) = fs::read_to_string("/proc/sys/kernel/osrelease") {
            return format!("linux {}", version.trim());
        }
    }
    format!("{} {}", env::consts::OS, env::consts::ARCH)
}
