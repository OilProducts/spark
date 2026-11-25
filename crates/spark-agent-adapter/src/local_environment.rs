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
        let mut environment = inherited_environment(self.environment_inheritance_policy);
        merge_env(&mut environment, env_vars.clone(), cfg!(windows));
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
        if let Some(rg_path) = find_executable_on_path("rg", None) {
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

/// What a child process inherits from Spark's own environment under `policy`.
pub(crate) fn inherited_environment(
    policy: EnvironmentInheritancePolicy,
) -> BTreeMap<String, String> {
    inherit(env::vars(), policy)
}

/// The variables of `vars` a child inherits under `policy`.
fn inherit(
    vars: impl Iterator<Item = (String, String)>,
    policy: EnvironmentInheritancePolicy,
) -> BTreeMap<String, String> {
    match policy {
        EnvironmentInheritancePolicy::InheritAll => vars.collect(),
        EnvironmentInheritancePolicy::InheritNone => BTreeMap::new(),
        EnvironmentInheritancePolicy::InheritCoreOnly => vars
            .filter(|(key, _)| is_core_env_key(key) && !is_sensitive_env_key(key))
            .collect(),
    }
}

/// Sets `vars` over `environment`. With `ignore_case`, as Windows treats
/// names, a caller's `PATH` replaces an inherited `Path` instead of both
/// reaching the child, where the inherited one could win.
pub(crate) fn merge_env<V>(
    environment: &mut BTreeMap<String, V>,
    vars: impl IntoIterator<Item = (String, V)>,
    ignore_case: bool,
) {
    for (key, value) in vars {
        if ignore_case {
            environment.retain(|existing, _| !existing.eq_ignore_ascii_case(&key));
        }
        environment.insert(key, value);
    }
}

/// Whether `key` names the search path: `PATH`, in any case on Windows.
pub(crate) fn is_path_key(key: &str) -> bool {
    names_var(key, "PATH")
}

/// Whether `key` names the variable `name`: in any case on Windows.
fn names_var(key: &str, name: &str) -> bool {
    if cfg!(windows) {
        key.eq_ignore_ascii_case(name)
    } else {
        key == name
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
/// be tied, it is stopped and the start fails.
pub fn spawn_process_group(command: Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        spawn_with_lifeline(command)
    }

    #[cfg(windows)]
    {
        let mut command = command;
        if let Some(error) = batch_directory_error(&command) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, error));
        }
        join_spark_job()?;
        command.spawn()
    }
}

/// Why Windows cannot run `command` where it asks: cmd.exe, which runs a batch
/// file, starts in the Windows directory instead of a UNC one, so the program
/// would work outside its project.
#[cfg_attr(not(windows), allow(dead_code))]
fn batch_directory_error(command: &Command) -> Option<String> {
    let program = Path::new(command.get_program());
    let directory = command.get_current_dir()?;
    let unc = matches!(
        directory.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..))
    );
    (unc && is_batch_file(program)).then(|| {
        format!(
            "{} is a batch file, which cmd.exe cannot run in the network directory {}: it \
             would start in the Windows directory instead. Open the project through a mapped \
             drive letter, or set the agent's program to its .exe.",
            program.display(),
            directory.display()
        )
    })
}

/// Whether Windows runs `program` through cmd.exe: a `.bat` or `.cmd` file.
pub(crate) fn is_batch_file(program: &Path) -> bool {
    program.extension().is_some_and(|extension| {
        ["bat", "cmd"]
            .iter()
            .any(|batch| extension.eq_ignore_ascii_case(batch))
    })
}

/// A pipe whose close-on-exec write end only this process holds, so its read
/// end sees end of input once this process has ended, however it ended.
#[cfg(unix)]
static SPARK_ALIVE: std::sync::OnceLock<(std::io::PipeReader, std::io::PipeWriter)> =
    std::sync::OnceLock::new();

/// Runs in the started process's group, with Spark's pipe on stdin and the read
/// end of the started process's own pipe on stdout. It forks a watcher and
/// returns at once. Once Spark's pipe closes, the watcher TERMs the group, then
/// KILLs it. Once every holder of the started process's pipe has gone (the
/// process and whatever it started that kept its descriptors), it goes too.
/// A TERM to the group ends it with the rest.
#[cfg(unix)]
const LIFELINE_SCRIPT: &str = "exec 3<&0 4<&1 </dev/null >/dev/null 2>&1; \
    { { read _ <&3; trap '' TERM; kill -TERM 0; sleep 2; kill -KILL 0; } & \
    read _ <&4; kill $!; } &";

// ponytail: Spark killed in the few milliseconds between a start and its
// lifeline, or midway through its own stop of a group (which ends the
// lifeline), leaves that group running; and a descendant that closes every
// inherited descriptor outlives its lifeline. Close these if they ever matter.
#[cfg(unix)]
#[allow(unsafe_code)]
fn spawn_with_lifeline(mut command: Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;

    let spark_alive = match SPARK_ALIVE.get() {
        Some(pipe) => pipe,
        None => {
            let pipe = std::io::pipe()?;
            SPARK_ALIVE.get_or_init(move || pipe)
        }
    };
    let (alive, holder) = std::io::pipe()?;
    // SAFETY: between fork and exec it only makes one system call. It leaves
    // the holder open across exec, so the program and what it starts hold it.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(&holder, rustix::io::FdFlags::empty())?;
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    // Spark's copy of the holder goes with the command.
    drop(command);
    let group = rustix::process::Pid::from_raw(child.id() as i32)
        .ok_or_else(|| std::io::Error::other("no process ID"))
        .and_then(|pid| Ok(rustix::process::getpgid(Some(pid))?));
    let tied = group.and_then(|group| {
        if group == rustix::process::getpgrp() {
            return Err(std::io::Error::other("it shares Spark's process group"));
        }
        let status = Command::new("/bin/sh")
            .args(["-c", LIFELINE_SCRIPT])
            .process_group(group.as_raw_pid())
            .current_dir("/")
            .stdin(spark_alive.0.try_clone()?)
            .stdout(alive)
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "its lifeline failed: {status}"
            )))
        }
    });
    match tied {
        Ok(()) => Ok(child),
        // Gone already, with nothing to tie.
        Err(_) if matches!(child.try_wait(), Ok(Some(_))) => Ok(child),
        Err(error) => {
            terminate_process_group(child.id(), Some(&mut child));
            Err(error)
        }
    }
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

/// `name` as a launch finds it: an explicit path (absolute, or with a
/// separator) as given if it is a file, else the first match on
/// `path_override` (else Spark's own `PATH`). A name with an extension is
/// matched as written. On Windows a bare name tries, in `PATHEXT`'s order,
/// each extension a launch can run before the name itself, so a `rg.cmd` shim
/// wins over an extensionless `rg` script and an npm `.ps1` shim hides nothing.
pub fn find_executable_on_path(
    name: &str,
    path_override: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let path_var = path_override
        .map(std::ffi::OsStr::to_os_string)
        .or_else(|| env::var_os("PATH"));
    find_executable_in_directories(name, path_var.iter().flat_map(env::split_paths))
}

/// [`find_executable_on_path`] over `directories` in order, each taken whole.
pub fn find_executable_in_directories(
    name: &str,
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    let pathext = cfg!(windows).then(own_pathext);
    find_executable_in(name, directories, pathext.as_deref())
}

/// Windows' `PATHEXT` where none is set.
const DEFAULT_PATHEXT: &str = ".EXE;.BAT;.CMD";

/// Spark's own `PATHEXT`, else Windows' default.
pub(crate) fn own_pathext() -> std::ffi::OsString {
    env::var_os("PATHEXT").unwrap_or_else(|| DEFAULT_PATHEXT.into())
}

/// The file `command`'s program names as its child finds it, by only the
/// `PATH` and `PATHEXT` `command` sets, whose whole environment its caller
/// sets after `env_clear`: an explicit path from the child's directory, else
/// the first match on `PATH`, whose relative entries resolve from that
/// directory too. Nothing the child does not receive, such as Spark's own
/// `PATH`, and no empty `PATH` entry makes a directory a search location.
pub(crate) fn child_program(command: &Command) -> Option<PathBuf> {
    let child_var = |name: &str| {
        command
            .get_envs()
            .filter(|(key, _)| key.to_str().is_some_and(|key| names_var(key, name)))
            .last()
            .and_then(|(_, value)| value)
            .map(std::ffi::OsStr::to_os_string)
    };
    let path = child_var("PATH").unwrap_or_default();
    let pathext =
        cfg!(windows).then(|| child_var("PATHEXT").unwrap_or_else(|| DEFAULT_PATHEXT.into()));
    let directory = command.get_current_dir().unwrap_or(Path::new(""));
    let program = Path::new(command.get_program());
    let program = if program.components().count() > 1 {
        directory.join(program)
    } else {
        program.to_path_buf()
    };
    find_executable_in(
        program.to_str()?,
        env::split_paths(&path)
            .filter(|entry| !entry.as_os_str().is_empty())
            .map(|entry| directory.join(entry)),
        pathext.as_deref(),
    )
}

/// `program` as a launch should name it. On Windows, where launching a bare
/// name tries only `.exe`, the match [`find_executable_on_path`] finds, such
/// as an npm `.cmd` shim. Elsewhere, and when nothing matches, `program` as
/// given: the system's own search resolves relative `PATH` entries from the
/// child's directory and skips files it cannot run, and a failure names
/// `program`.
pub fn launch_program(program: PathBuf) -> PathBuf {
    if cfg!(windows) {
        if let Some(found) = program
            .to_str()
            .and_then(|name| find_executable_on_path(name, None))
        {
            return found;
        }
    }
    program
}

/// [`find_executable_in_directories`] with `pathext` given, `None` outside
/// Windows.
fn find_executable_in(
    name: &str,
    directories: impl IntoIterator<Item = PathBuf>,
    pathext: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() || candidate.components().count() > 1 {
        return is_executable(candidate).then(|| candidate.to_path_buf());
    }
    let pathext = pathext
        .map(std::ffi::OsStr::to_string_lossy)
        .unwrap_or_default();
    // A launch runs an image or, through cmd.exe, a batch file; other
    // extensions, such as an npm `.ps1` shim's, cannot start.
    let names = pathext
        .split(';')
        .filter(|extension| {
            candidate.extension().is_none()
                && [".com", ".exe", ".bat", ".cmd"]
                    .iter()
                    .any(|runnable| extension.eq_ignore_ascii_case(runnable))
        })
        .map(|extension| format!("{name}{extension}"))
        .chain([name.to_string()])
        .collect::<Vec<_>>();
    // An empty entry would search the launch's own directory.
    directories
        .into_iter()
        .filter(|directory| !directory.as_os_str().is_empty())
        .find_map(|directory| {
            names
                .iter()
                .map(|name| directory.join(name))
                .find(|candidate| is_executable(candidate))
        })
}

/// Whether `path` is a file a launch can run: elsewhere than Windows, one
/// with an execute bit, as the system's own search requires.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn absolutize(path: &Path) -> PathBuf {
    let expanded = expand_home(path, spark_common::paths::user_home_dir());
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

/// Expands a leading `~` component with `home`. `\` separates only on
/// Windows, so elsewhere `~\repo` is a literal relative name.
fn expand_home(path: &Path, home: Option<PathBuf>) -> PathBuf {
    let mut components = path.components();
    match (components.next(), home) {
        (Some(Component::Normal(first)), Some(home)) if first == "~" => home.join(components),
        _ => path.to_path_buf(),
    }
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
        "COMPUTERNAME",
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
        "NUMBER_OF_PROCESSORS",
        "NVM_DIR",
        "NODE_PATH",
        "OLDPWD",
        "OS",
        "PATH",
        "PATHEXT",
        "PNPM_HOME",
        "PKG_CONFIG_PATH",
        "PROCESSOR_ARCHITECTURE",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PSMODULEPATH",
        "PYTHONHOME",
        "PYTHONPATH",
        "PWD",
        "RUSTUP_HOME",
        "SDKMAN_DIR",
        "SHELL",
        "SYSTEMDRIVE",
        "SYSTEMROOT",
        "TEMP",
        "TERM",
        "TMP",
        "TMPDIR",
        "USER",
        "USERNAME",
        "USERPROFILE",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_leading_tilde_component_is_the_home() {
        let home = env::temp_dir().join("home");
        let expand = |path: &str| expand_home(Path::new(path), Some(home.clone()));
        assert_eq!(expand("~/repo"), home.join("repo"));
        // `\` separates only on Windows.
        let backslash = if cfg!(windows) {
            home.join("repo")
        } else {
            PathBuf::from(r"~\repo")
        };
        assert_eq!(expand(r"~\repo"), backslash);
        assert_eq!(expand("~repo"), PathBuf::from("~repo"));
        assert_eq!(
            expand_home(Path::new("~/repo"), None),
            PathBuf::from("~/repo")
        );
    }

    /// An empty file a launch can run, with an execute bit outside Windows.
    fn executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
        #[cfg(unix)]
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_pathext_shim_wins_over_an_extensionless_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (first, second) = (temp.path().join("first"), temp.path().join("second"));
        for file in [
            first.join("rg"),
            first.join("rg.ps1"),
            first.join("rg.cmd"),
            first.join("rg.cmd.exe"),
            second.join("rg.exe"),
        ] {
            executable(&file);
        }
        let path = env::join_paths([temp.path(), &first, &second]).unwrap();
        // Lowercase, so a case-insensitive file system returns the files as named.
        let pathext = |pathext| Some(std::ffi::OsStr::new(pathext));
        let windows = pathext(".com;.exe;.ps1;.bat;.cmd");
        let find = |name: &str, pathext| find_executable_in(name, env::split_paths(&path), pathext);

        // Windows runs the shim beside the script, past an npm PowerShell
        // shim a launch cannot start; elsewhere the script runs.
        assert_eq!(find("rg", windows), Some(first.join("rg.cmd")));
        assert_eq!(find("rg", None), Some(first.join("rg")));
        // A name with an extension is taken as written, whatever PATHEXT holds.
        assert_eq!(find("rg.exe", windows), Some(second.join("rg.exe")));
        assert_eq!(find("rg.cmd", pathext(".exe")), Some(first.join("rg.cmd")));
        // Explicit paths resolve as given, never through PATH, and elsewhere
        // than Windows only to a file with an execute bit.
        let explicit = second.join("rg.exe");
        assert_eq!(
            find(explicit.to_str().unwrap(), windows),
            Some(explicit.clone())
        );
        assert_eq!(find("first/rg", windows), None);
        assert_eq!(find("src/lib.rs", None).is_some(), cfg!(windows));
    }

    /// A child's program is found by only the `PATH` and `PATHEXT` it
    /// receives, in any spelling on Windows, and an explicit relative path
    /// from its directory, which no empty entry adds to the search.
    #[test]
    fn child_program_follows_the_child_environment_and_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join("bin");
        for file in [
            temp.path().join("agent"),
            bin.join("agent"),
            bin.join("agent.bat"),
            bin.join("agent.cmd"),
        ] {
            executable(&file);
        }

        let mut explicit = Command::new(Path::new("bin").join("agent"));
        explicit.current_dir(temp.path());
        assert_eq!(child_program(&explicit), Some(bin.join("agent")));

        // Lowercase, so a case-insensitive file system returns the files as named.
        let mut bare = Command::new("agent");
        bare.current_dir(temp.path())
            .env_clear()
            .env("PATH", env::join_paths(["", "bin"]).unwrap())
            .env("Pathext", ".cmd;.bat");
        let found = if cfg!(windows) { "agent.cmd" } else { "agent" };
        assert_eq!(child_program(&bare), Some(bin.join(found)));

        // Without a PATH of its own the child finds nothing, not the work
        // area's program nor one on Spark's own PATH.
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(find_executable_on_path(shell, None).is_some());
        for name in ["agent", shell] {
            let mut unset = Command::new(name);
            unset.current_dir(temp.path()).env_clear();
            assert_eq!(child_program(&unset), None, "{name}");
        }
    }

    /// cmd.exe would start a batch file in the Windows directory instead of a
    /// UNC one, so the launch fails first; elsewhere it goes ahead.
    #[cfg(windows)]
    #[test]
    fn a_batch_file_never_starts_outside_its_unc_directory() {
        let command = |program: &str, directory: &str| {
            let mut command = Command::new(program);
            command.current_dir(directory);
            command
        };
        let error = spawn_process_group(command(r"C:\tools\claude.cmd", r"\\server\share\repo"))
            .unwrap_err();
        assert!(error.to_string().contains("mapped drive letter"), "{error}");
        for (program, directory) in [
            (r"C:\tools\claude.exe", r"\\server\share\repo"),
            (r"C:\tools\claude.cmd", r"C:\repo"),
        ] {
            assert_eq!(batch_directory_error(&command(program, directory)), None);
        }
    }

    #[test]
    fn caller_variables_replace_inherited_ones_in_any_case_only_on_windows() {
        let inherited = || {
            BTreeMap::from([
                ("Path".to_string(), "inherited".to_string()),
                ("HOME".to_string(), "home".to_string()),
            ])
        };
        let caller = BTreeMap::from([("PATH".to_string(), "caller".to_string())]);

        let mut windows = inherited();
        merge_env(&mut windows, caller.clone(), true);
        assert_eq!(
            windows,
            BTreeMap::from([
                ("HOME".to_string(), "home".to_string()),
                ("PATH".to_string(), "caller".to_string()),
            ])
        );

        let mut unix = inherited();
        merge_env(&mut unix, caller, false);
        assert_eq!(unix.get("Path").map(String::as_str), Some("inherited"));
        assert_eq!(unix.get("PATH").map(String::as_str), Some("caller"));
    }

    #[test]
    fn inherit_core_only_keeps_windows_system_variables() {
        // As Windows spells them.
        let windows = [
            "APPDATA",
            "COMPUTERNAME",
            "ComSpec",
            "HOMEDRIVE",
            "HOMEPATH",
            "LOCALAPPDATA",
            "NUMBER_OF_PROCESSORS",
            "OS",
            "Path",
            "PATHEXT",
            "PROCESSOR_ARCHITECTURE",
            "ProgramData",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "PSModulePath",
            "SystemDrive",
            "SystemRoot",
            "TEMP",
            "TMP",
            "USERNAME",
            "USERPROFILE",
            "windir",
        ]
        .map(|key| (key.to_string(), format!("{key}-value")));
        assert_eq!(
            inherit(
                windows.clone().into_iter(),
                EnvironmentInheritancePolicy::InheritCoreOnly
            ),
            BTreeMap::from(windows)
        );
    }
}
