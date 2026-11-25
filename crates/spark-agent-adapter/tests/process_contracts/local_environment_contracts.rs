use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use spark_agent_adapter::{
    CommandOptions, DirEntry, EnvironmentInheritancePolicy, EnvironmentResult, ExecResult,
    ExecutionEnvironment, ExecutionEnvironmentBackend, GrepOptions, LocalExecutionEnvironment,
    ToolDispatchContext,
};
use tempfile::tempdir;
use unified_llm_adapter::ToolCall;

use super::test_support::ENV_LOCK;

#[test]
fn local_environment_resolves_paths_and_handles_file_directory_lifecycle() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    let environment = ExecutionEnvironment::local(&workspace);

    assert_eq!(environment.working_directory(), workspace.to_string_lossy());
    assert!(!workspace.exists());
    environment.initialize().expect("initialize");
    assert!(workspace.is_dir());
    assert!(matches!(
        environment.platform().as_str(),
        "linux" | "darwin" | "windows" | "wasm"
    ));
    assert!(!environment.os_version().is_empty());

    environment
        .write_file("nested/example.txt", "first line\nsecond line\n")
        .expect("write file");
    assert!(environment.file_exists("nested/example.txt"));
    assert!(environment.is_directory("nested"));
    assert_eq!(
        environment
            .read_file("nested/example.txt", None, None)
            .expect("read file"),
        "first line\nsecond line\n"
    );
    assert_eq!(
        environment
            .read_file("nested/example.txt", Some(2), Some(1))
            .expect("read slice"),
        "second line\n"
    );

    environment
        .write_file("nested/rename_me.txt", "rename me\n")
        .expect("write rename source");
    environment
        .rename_file("nested/rename_me.txt", "nested/renamed.txt")
        .expect("rename");
    assert!(!environment.file_exists("nested/rename_me.txt"));
    assert_eq!(
        environment
            .read_file("nested/renamed.txt", None, None)
            .expect("read renamed"),
        "rename me\n"
    );
    environment
        .delete_file("nested/renamed.txt")
        .expect("delete renamed");
    assert!(!environment.file_exists("nested/renamed.txt"));

    assert_eq!(
        environment.list_directory(".", 1).expect("list directory"),
        vec![
            DirEntry {
                name: "nested".to_string(),
                is_dir: true,
                size: None,
            },
            DirEntry {
                name: "nested/example.txt".to_string(),
                is_dir: false,
                size: Some(23),
            },
        ]
    );
}

/// Prints the environment as `KEY=VALUE` lines, in the platform's shell.
#[cfg(windows)]
const PRINT_ENV: &str = "set";
#[cfg(not(windows))]
const PRINT_ENV: &str = "env";

fn printed_env(result: &ExecResult) -> BTreeMap<String, String> {
    env_lines(&result.stdout)
}

/// `KEY=VALUE` lines as a map.
pub(super) fn env_lines(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn local_environment_exec_command_captures_streams_timeout_and_filters_environment() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tempdir = tempdir().expect("tempdir");
    let old_password = std::env::var_os("Unit_Test_PaSsWoRd");
    let old_visible = std::env::var_os("UNIT_TEST_VISIBLE");
    std::env::set_var("Unit_Test_PaSsWoRd", "shh");
    std::env::set_var("UNIT_TEST_VISIBLE", "visible");
    // The same steps in each platform's shell: cmd.exe on Windows, bash elsewhere.
    #[cfg(windows)]
    let (streams, streamed, sleep, started) = (
        "echo out& (echo err)1>&2& exit 7",
        ("out\r\n", "err\r\n"),
        "echo start& ping -n 61 127.0.0.1 >nul",
        "start\r\n",
    );
    #[cfg(not(windows))]
    let (streams, streamed, sleep, started) = (
        "printf 'out\\n'; printf 'err\\n' >&2; exit 7",
        ("out\n", "err\n"),
        "printf 'start\\n'; sleep 60",
        "start\n",
    );

    let environment = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        5_000,
        60_000,
        EnvironmentInheritancePolicy::InheritCoreOnly,
    )
    .into_execution_environment();

    let result = environment
        .exec_command(
            streams,
            CommandOptions {
                timeout_ms: Some(60_000),
                ..CommandOptions::default()
            },
        )
        .expect("exec command");
    assert_eq!((result.stdout.as_str(), result.stderr.as_str()), streamed);
    assert_eq!(result.exit_code, 7);
    assert!(!result.timed_out);
    assert!(result.duration_ms < 60_000);

    let explicit = CommandOptions {
        timeout_ms: Some(60_000),
        env_vars: BTreeMap::from([("EXPLICIT_VALUE".to_string(), "from-env-vars".to_string())]),
        ..CommandOptions::default()
    };
    let core_only = printed_env(
        &environment
            .exec_command(PRINT_ENV, explicit.clone())
            .expect("env command"),
    );
    assert!(
        !core_only.contains_key("Unit_Test_PaSsWoRd"),
        "{core_only:?}"
    );
    assert!(
        !core_only.contains_key("UNIT_TEST_VISIBLE"),
        "{core_only:?}"
    );
    assert_eq!(core_only["EXPLICIT_VALUE"], "from-env-vars");
    assert!(
        core_only.keys().any(|key| key.eq_ignore_ascii_case("PATH")),
        "{core_only:?}"
    );

    let inherit_all = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        5_000,
        60_000,
        EnvironmentInheritancePolicy::InheritAll,
    )
    .into_execution_environment();
    let all = printed_env(
        &inherit_all
            .exec_command(PRINT_ENV, explicit)
            .expect("inherit all env command"),
    );
    assert_eq!(
        [
            &all["Unit_Test_PaSsWoRd"],
            &all["UNIT_TEST_VISIBLE"],
            &all["EXPLICIT_VALUE"]
        ],
        ["shh", "visible", "from-env-vars"]
    );

    let timeout_result = environment
        .exec_command(sleep, CommandOptions::default())
        .expect("timeout command");
    assert!(timeout_result.timed_out);
    assert_eq!(timeout_result.stdout, started);
    assert_eq!(timeout_result.stderr, timeout_message(5_000));
    assert_ne!(timeout_result.exit_code, 0);

    restore_env("Unit_Test_PaSsWoRd", old_password);
    restore_env("UNIT_TEST_VISIBLE", old_visible);
}

#[test]
fn local_environment_exec_command_inherit_none_omits_parent_environment_but_keeps_explicit_vars() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tempdir = tempdir().expect("tempdir");
    let old_password = std::env::var_os("Unit_Test_PaSsWoRd");
    let old_visible = std::env::var_os("UNIT_TEST_VISIBLE");
    std::env::set_var("Unit_Test_PaSsWoRd", "shh");
    std::env::set_var("UNIT_TEST_VISIBLE", "visible");

    let environment = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        5_000,
        60_000,
        EnvironmentInheritancePolicy::InheritNone,
    )
    .into_execution_environment();

    let result = environment
        .exec_command(
            PRINT_ENV,
            CommandOptions {
                timeout_ms: Some(60_000),
                env_vars: BTreeMap::from([(
                    "EXPLICIT_VALUE".to_string(),
                    "from-env-vars".to_string(),
                )]),
                ..CommandOptions::default()
            },
        )
        .expect("inherit-none env command");

    restore_env("Unit_Test_PaSsWoRd", old_password);
    restore_env("UNIT_TEST_VISIBLE", old_visible);

    let env = printed_env(&result);
    assert!(!env.contains_key("Unit_Test_PaSsWoRd"), "{env:?}");
    assert!(!env.contains_key("UNIT_TEST_VISIBLE"), "{env:?}");
    assert_eq!(env["EXPLICIT_VALUE"], "from-env-vars");
    assert_eq!(result.stderr, "");
    assert_eq!(result.exit_code, 0);
    assert!(!result.timed_out);
}

/// Every Windows system variable on the core allowlist that Spark has
/// reaches a child under `InheritCoreOnly`, on any platform, since the list
/// is the same everywhere.
#[test]
fn local_environment_inherit_core_only_passes_windows_system_variables_to_the_child() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tempdir = tempdir().expect("tempdir");
    let keys = [
        "APPDATA",
        "COMPUTERNAME",
        "COMSPEC",
        "HOMEDRIVE",
        "HOMEPATH",
        "LOCALAPPDATA",
        "NUMBER_OF_PROCESSORS",
        "OS",
        "PATHEXT",
        "PROCESSOR_ARCHITECTURE",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PSMODULEPATH",
        "SYSTEMDRIVE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "USERNAME",
        "USERPROFILE",
        "WINDIR",
    ];
    // Windows' own values are checked as they are, since its shell needs
    // some, such as `COMSPEC`; elsewhere each missing key is set for the check.
    let mut missing = Vec::new();
    let expected = keys
        .into_iter()
        .filter_map(|key| match std::env::var_os(key) {
            Some(value) => Some((key, value.to_string_lossy().into_owned())),
            None if cfg!(windows) => None,
            None => {
                missing.push(key);
                let value = format!("{key}-value");
                std::env::set_var(key, &value);
                Some((key, value))
            }
        })
        .collect::<Vec<_>>();

    // Windows' `set` writes a pipe in the console's code page, so there
    // `cmd /U` writes UTF-16 to a file instead, which keeps a value such as
    // `C:\Users\René` exact.
    #[cfg(windows)]
    let dump_env = "cmd /U /C set > env.txt";
    #[cfg(not(windows))]
    let dump_env = "env > env.txt";
    let result = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        5_000,
        60_000,
        EnvironmentInheritancePolicy::InheritCoreOnly,
    )
    .into_execution_environment()
    .exec_command(
        dump_env,
        CommandOptions {
            timeout_ms: Some(60_000),
            ..CommandOptions::default()
        },
    );
    for key in missing {
        std::env::remove_var(key);
    }

    let result = result.expect("env command");
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let dump = std::fs::read(tempdir.path().join("env.txt")).expect("environment dump");
    #[cfg(windows)]
    let dump = String::from_utf16(
        &dump
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect::<Vec<_>>(),
    );
    #[cfg(not(windows))]
    let dump = String::from_utf8(dump);
    let env = env_lines(&dump.expect("environment dump text"));
    for (key, value) in expected {
        // Windows spells some differently, such as `SystemRoot`.
        let received = env
            .iter()
            .find(|(name, _)| {
                if cfg!(windows) {
                    name.eq_ignore_ascii_case(key)
                } else {
                    name.as_str() == key
                }
            })
            .map(|(_, value)| value.as_str());
        assert_eq!(received, Some(value.as_str()), "{key}: {env:?}");
    }
}

/// Windows names are case-insensitive, so a caller's `PATH` replaces the
/// inherited `Path`; elsewhere `Path` is another variable and stays.
#[test]
fn local_environment_caller_path_replaces_inherited_path_in_any_case_only_on_windows() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tempdir = tempdir().expect("tempdir");
    let original = std::env::var_os("PATH").expect("PATH");
    // Windows keeps its search path as `Path`, so the inheritance is spelled so.
    #[cfg(windows)]
    {
        std::env::remove_var("PATH");
        std::env::set_var("Path", &original);
    }
    #[cfg(not(windows))]
    std::env::set_var("Path", "inherited-mixed-case");
    let caller_path = std::env::join_paths(
        std::iter::once(tempdir.path().to_path_buf()).chain(std::env::split_paths(&original)),
    )
    .expect("caller PATH")
    .into_string()
    .expect("UTF-8 PATH");

    let result = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        5_000,
        60_000,
        EnvironmentInheritancePolicy::InheritCoreOnly,
    )
    .into_execution_environment()
    .exec_command(
        PRINT_ENV,
        CommandOptions {
            timeout_ms: Some(60_000),
            env_vars: BTreeMap::from([("PATH".to_string(), caller_path.clone())]),
            ..CommandOptions::default()
        },
    );

    #[cfg(windows)]
    std::env::set_var("Path", &original);
    #[cfg(not(windows))]
    std::env::remove_var("Path");

    let env = printed_env(&result.expect("env command"));
    let paths = env
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    #[cfg(windows)]
    assert_eq!(
        paths.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
        [caller_path.as_str()]
    );
    #[cfg(not(windows))]
    assert_eq!(
        paths,
        [
            ("PATH", caller_path.as_str()),
            ("Path", "inherited-mixed-case")
        ]
    );
}

#[cfg(not(windows))]
#[test]
fn local_environment_exec_command_uses_bash_on_unix() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());

    let result = environment
        .exec_command(
            "[[ -n \"$BASH_VERSION\" ]] && printf 'bash:%s' \"$BASH_VERSION\"",
            CommandOptions {
                timeout_ms: Some(60_000),
                ..CommandOptions::default()
            },
        )
        .expect("bash-specific command");

    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.starts_with("bash:"), "{result:?}");
    assert_eq!(result.stderr, "");
    assert!(!result.timed_out);
}

#[cfg(unix)]
#[test]
fn local_environment_exec_command_kills_child_that_survives_sigterm() {
    // The command inherits PATH, which other tests swap out under this lock.
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tempdir = tempdir().expect("tempdir");
    let marker = tempdir.path().join("survived.txt");
    let environment = LocalExecutionEnvironment::with_options(
        tempdir.path(),
        50,
        50,
        EnvironmentInheritancePolicy::InheritCoreOnly,
    )
    .into_execution_environment();

    let started = Instant::now();
    let result = environment
        .exec_command(
            "(trap '' TERM; sleep 3; printf survived > \"$MARKER\") & sleep 5",
            CommandOptions {
                env_vars: BTreeMap::from([(
                    "MARKER".to_string(),
                    marker.to_string_lossy().to_string(),
                )]),
                ..CommandOptions::default()
            },
        )
        .expect("timeout command");

    assert!(result.timed_out, "{result:?}");
    assert_eq!(result.stderr, timeout_message(50));
    assert_ne!(result.exit_code, 0);

    let child_write_time = started + Duration::from_millis(3_500);
    while Instant::now() < child_write_time {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!marker.exists(), "SIGTERM-ignoring child escaped SIGKILL");
}

#[test]
fn local_environment_grep_and_glob_return_environment_relative_results() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file("notes.txt", "Alpha\nignored\nALPHA\n")
        .expect("write notes");
    environment
        .write_file("nested/ignore.md", "alpha\n")
        .expect("write nested");

    let grep_result = environment
        .grep(
            "alpha",
            ".",
            &GrepOptions {
                glob_filter: Some("*.txt".to_string()),
                case_insensitive: true,
                max_results: 2,
            },
        )
        .expect("grep");
    assert_eq!(
        grep_result.lines().collect::<Vec<_>>(),
        vec!["notes.txt:1:Alpha", "notes.txt:3:ALPHA"]
    );

    let old_file = tempdir.path().join("alpha.txt");
    let new_file = tempdir.path().join("omega.txt");
    environment.write_file(&old_file, "old").expect("write old");
    environment.write_file(&new_file, "new").expect("write new");
    let old_time = filetime::FileTime::from_unix_time(1, 0);
    let new_time = filetime::FileTime::from_unix_time(2, 0);
    let notes_time = filetime::FileTime::from_unix_time(0, 0);
    filetime::set_file_mtime(tempdir.path().join("notes.txt"), notes_time).expect("notes mtime");
    filetime::set_file_mtime(&old_file, old_time).expect("old mtime");
    filetime::set_file_mtime(&new_file, new_time).expect("new mtime");

    assert_eq!(
        environment.glob("*.txt", ".").expect("glob"),
        vec!["omega.txt", "alpha.txt", "notes.txt"]
    );
}

#[test]
fn execution_environment_accepts_consumer_provided_backends_and_builtin_tools_use_it() {
    let backend = RecordingEnvironment::default();
    let calls = backend.calls.clone();
    let environment = ExecutionEnvironment::from_backend(backend);
    let registry = spark_agent_adapter::create_openai_profile("gpt-5.2").registry();

    let write_result = registry.dispatch(
        ToolCall::new(
            "call-write",
            "write_file",
            json!({"path": "notes.txt", "content": "alpha\n"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!write_result.is_error);
    assert_eq!(
        write_result.content,
        json!({"path": "notes.txt", "bytes_written": 6})
    );

    let shell_result = registry.dispatch(
        ToolCall::new("call-shell", "shell", json!({"command": "echo hello"})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(!shell_result.is_error);
    assert_eq!(shell_result.content["stdout"], json!("echo hello"));

    assert_eq!(
        calls.lock().expect("calls").as_slice(),
        [
            "write_file:notes.txt:alpha\n",
            "exec_command:echo hello:10000"
        ]
    );
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

fn timeout_message(timeout_ms: u64) -> String {
    format!(
        "[ERROR: Command timed out after {timeout_ms}ms. Partial output is shown above.\n\
You can retry with a longer timeout by setting the timeout_ms parameter.]"
    )
}

#[derive(Debug, Default)]
struct RecordingEnvironment {
    calls: Arc<Mutex<Vec<String>>>,
}

impl ExecutionEnvironmentBackend for RecordingEnvironment {
    fn read_file(
        &self,
        path: &Path,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        Ok(format!("content:{}", path.display()))
    }

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>> {
        Ok(format!("content:{}", path.display()).into_bytes())
    }

    fn write_file(&self, path: &Path, content: &str) -> EnvironmentResult<()> {
        self.calls
            .lock()
            .expect("calls")
            .push(format!("write_file:{}:{content}", path.display()));
        Ok(())
    }

    fn file_exists(&self, _path: &Path) -> bool {
        true
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
        command: &str,
        options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        self.calls.lock().expect("calls").push(format!(
            "exec_command:{command}:{}",
            options.timeout_ms.unwrap_or_default()
        ));
        Ok(ExecResult {
            stdout: command.to_string(),
            stderr: String::new(),
            exit_code: 0,
            timed_out: false,
            duration_ms: 1,
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
        ".".to_string()
    }

    fn platform(&self) -> String {
        "test".to_string()
    }

    fn os_version(&self) -> String {
        "1.0".to_string()
    }
}

/// The command watches its abort signal itself, so an abort whose cleanup
/// ran before the command registered still kills it.
#[test]
fn an_aborted_command_is_killed_without_waiting_for_cleanup() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    let abort = unified_llm_adapter::AbortSignal::new();
    let marker = tempdir.path().join("started");
    thread::spawn({
        let abort = abort.clone();
        move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !marker.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            abort.abort("Stopped.");
        }
    });

    // The same steps in each platform's shell: cmd.exe on Windows, bash elsewhere.
    #[cfg(windows)]
    let command = "type nul > started && ping -n 31 127.0.0.1 >nul";
    #[cfg(not(windows))]
    let command = "touch started && sleep 30";

    let started = Instant::now();
    let result = environment
        .exec_command(
            command,
            CommandOptions {
                timeout_ms: Some(60_000),
                abort_signal: Some(abort),
                ..CommandOptions::default()
            },
        )
        .expect("exec command");

    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(tempdir.path().join("started").exists());
    assert!(result.stderr.ends_with("Stopped."), "{result:?}");
    assert!(!result.timed_out);
}
