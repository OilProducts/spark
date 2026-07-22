#![forbid(unsafe_code)]

//! `spark-server` binary shell for the Rust rewrite.
//!
//! M1 implements only command-shell compatibility, source-checkout guard
//! wiring, a bounded development init path, and the hidden worker command
//! surface needed by later local-container execution work.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use spark_common::debug::{codex_jsonrpc_trace_enabled_with_env, ENV_SPARK_DEBUG_CODEX_JSONRPC};
use spark_common::logging::init_spark_logging;
use spark_common::paths::{Environment, ProcessEnvironment};
use spark_common::settings::{
    resolve_settings_with_env, validate_settings, SettingsOverrides, SparkSettings,
};
use spark_common::source_checkout::{
    installed_package_root_from_executable, require_explicit_dev_home_with_env,
    source_checkout_root_from_manifest,
};
use spark_common::SparkCommonError;
use tracing::Level;

const EXIT_GENERAL_FAILURE: i32 = 1;
const EXIT_USAGE_ERROR: i32 = 2;
const SERVICE_UNIT_NAME: &str = "spark.service";

const TOP_LEVEL_HELP: &str = concat!(
    "usage: spark-server [-h] {serve,init,service} ...\n",
    "\n",
    "Spark operator CLI\n",
    "\n",
    "positional arguments:\n",
    "  {serve,init,service}\n",
    "    serve               Start the Spark API server\n",
    "    init                Initialize Spark runtime directories and seed packaged\n",
    "                        flows\n",
    "    service             Manage the installed Spark user service\n",
    "\n",
    "options:\n",
    "  -h, --help            show this help message and exit\n",
);

const WORKER_RUN_NODE_HELP: &str = concat!(
    "usage: spark-server worker run-node [-h]\n",
    "\n",
    "options:\n",
    "  -h, --help  show this help message and exit\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    fn stdout(exit_code: i32, stdout: impl Into<String>) -> Self {
        Self {
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn stderr(exit_code: i32, stderr: impl Into<String>) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct InitArgs {
    data_dir: Option<PathBuf>,
    flows_dir: Option<PathBuf>,
    force: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ServeArgs {
    data_dir: Option<PathBuf>,
    flows_dir: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
    host: String,
    port: u16,
    reload: bool,
    debug_codex_jsonrpc: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ServiceInstallArgs {
    data_dir: Option<PathBuf>,
    flows_dir: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedStarterFlowsResult {
    pub flows_dir: PathBuf,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeConfiguration {
    pub host: String,
    pub port: u16,
    pub reload: bool,
    pub debug_codex_jsonrpc: bool,
    pub settings: SparkSettings,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuntimeInitializationOptions {
    pub data_dir: Option<PathBuf>,
    pub flows_dir: Option<PathBuf>,
    pub ui_dir: Option<PathBuf>,
    pub force: bool,
}

/// Runs the `spark-server` command and writes process output.
pub fn run() -> i32 {
    let _ = init_spark_logging(Level::INFO);
    let args = std::env::args_os().collect::<Vec<_>>();
    let stripped = strip_program_name(args.clone(), "spark-server");
    if stripped.first().map(String::as_str) == Some("worker")
        && stripped.get(1).map(String::as_str) == Some("run-node")
        && !stripped.iter().skip(2).any(|arg| is_help_arg(arg))
    {
        return attractor_execution::run_worker_node_from_reader_writer(
            std::io::stdin().lock(),
            std::io::stdout(),
            attractor_runtime_handler_runner(),
        );
    }
    if stripped.first().map(String::as_str) == Some("serve")
        && !stripped.iter().skip(1).any(|arg| is_help_arg(arg))
    {
        match run_serve_process(&stripped, &ProcessEnvironment) {
            Ok(()) => return 0,
            Err(output) => {
                write_process_output(&output);
                return output.exit_code;
            }
        }
    }
    let output = run_with_args(args);
    write_process_output(&output);
    output.exit_code
}

pub fn run_with_args<I, S>(args: I) -> CommandOutput
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    run_with_args_and_env(args, &ProcessEnvironment)
}

pub fn run_with_args_and_env<I, S, E>(args: I, env: &E) -> CommandOutput
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    E: Environment,
{
    let args = strip_program_name(args, "spark-server");
    run_server_shell(&args, env)
}

fn run_server_shell(args: &[String], env: &impl Environment) -> CommandOutput {
    if args.is_empty() || is_help_arg(&args[0]) {
        return CommandOutput::stdout(0, TOP_LEVEL_HELP);
    }

    match args[0].as_str() {
        "serve" => run_serve_shell(args, env),
        "init" => run_init_shell(args, env),
        "service" => run_service_shell(args, env),
        "worker" => run_worker_shell(args),
        _ => usage_error(format!("argument command: invalid choice: '{}'", args[0])),
    }
}

fn run_serve_shell(args: &[String], env: &impl Environment) -> CommandOutput {
    match build_serve_configuration_from_args(args, env) {
        Ok(config) => CommandOutput::stdout(
            0,
            format!(
                "spark-server serve configured for {}:{}\n",
                config.host, config.port
            ),
        ),
        Err(output) => output,
    }
}

fn run_init_shell(args: &[String], env: &impl Environment) -> CommandOutput {
    let init_args = match parse_init_args(&args[1..]) {
        Ok(args) => args,
        Err(message) => return usage_error(message),
    };
    if let Some(guard_output) =
        enforce_runtime_home_guard("spark-server init", init_args.data_dir.as_deref(), env)
    {
        return guard_output;
    }

    match initialize_runtime(&init_args, env) {
        Ok((settings, result)) => CommandOutput::stdout(
            0,
            format!(
                "Initialized Spark at {}\nSeeded flows: {}\ncreated={} updated={} skipped={}\n",
                settings.data_dir.display(),
                result.flows_dir.display(),
                result.created.len(),
                result.updated.len(),
                result.skipped.len()
            ),
        ),
        Err(message) => CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    }
}

fn run_service_shell(args: &[String], env: &impl Environment) -> CommandOutput {
    let Some(service_command) = args.get(1).map(String::as_str) else {
        return usage_error("service requires a subcommand");
    };

    match service_command {
        "install" => {
            let install_args = match parse_service_install_args(&args[2..]) {
                Ok(args) => args,
                Err(message) => return usage_error(message),
            };
            if let Some(guard_output) = enforce_runtime_home_guard(
                "spark-server service install",
                install_args.data_dir.as_deref(),
                env,
            ) {
                return guard_output;
            }
            run_service_install(&install_args, env)
        }
        "remove" => run_service_remove(env),
        "status" => run_service_status(env),
        _ => usage_error("service requires a subcommand"),
    }
}

fn run_worker_shell(args: &[String]) -> CommandOutput {
    let Some(worker_command) = args.get(1).map(String::as_str) else {
        return usage_error("worker requires a subcommand");
    };
    match worker_command {
        "run-node" => {
            if args.iter().skip(2).any(|arg| is_help_arg(arg)) {
                return CommandOutput::stdout(0, WORKER_RUN_NODE_HELP);
            }
            let stdout = SharedOutput::default();
            let exit_code = attractor_execution::run_worker_node_from_reader_writer(
                std::io::Cursor::new(Vec::<u8>::new()),
                stdout.clone(),
                attractor_runtime_handler_runner(),
            );
            let stdout_text = String::from_utf8_lossy(&stdout.0.lock().unwrap()).into_owned();
            CommandOutput {
                exit_code,
                stdout: stdout_text,
                stderr: String::new(),
            }
        }
        _ => usage_error("worker requires a subcommand"),
    }
}

#[derive(Clone, Default)]
struct SharedOutput(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for SharedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn attractor_runtime_handler_runner() -> attractor_runtime::RuntimeHandlerRunner {
    attractor_runtime::RuntimeHandlerRunner::new().with_rust_llm_client(rust_llm_client_from_env())
}

fn rust_llm_client_from_env() -> unified_llm_adapter::Client {
    match resolve_server_settings_with_env(&SettingsOverrides::default(), &ProcessEnvironment) {
        Ok(settings) => rust_llm_client_from_settings(&settings),
        Err(error) => {
            tracing::warn!(
                target: "spark_server",
                error = %error,
                "failed to resolve Spark settings for unified LLM profiles"
            );
            unified_llm_adapter::Client::from_env().unwrap_or_else(|error| {
                tracing::warn!(
                    target: "spark_server",
                    error = %error,
                    "failed to initialize unified LLM client from environment"
                );
                unified_llm_adapter::Client::new()
            })
        }
    }
}

pub fn rust_llm_client_from_settings(settings: &SparkSettings) -> unified_llm_adapter::Client {
    let env = std::env::vars().collect::<std::collections::BTreeMap<_, _>>();
    unified_llm_adapter::Client::from_env_map_and_profiles(&env, &settings.config_dir, None)
        .unwrap_or_else(|error| {
            tracing::warn!(
                target: "spark_server",
                error = %error,
                "failed to initialize unified LLM client from environment and profiles"
            );
            unified_llm_adapter::Client::new()
        })
}

fn initialize_runtime(
    args: &InitArgs,
    env: &impl Environment,
) -> std::result::Result<(SparkSettings, SeedStarterFlowsResult), String> {
    initialize_runtime_with_options(
        &RuntimeInitializationOptions {
            data_dir: args.data_dir.clone(),
            flows_dir: args.flows_dir.clone(),
            ui_dir: None,
            force: args.force,
        },
        env,
    )
}

fn initialize_runtime_with_ui(
    args: &InitArgs,
    ui_dir: Option<PathBuf>,
    env: &impl Environment,
) -> std::result::Result<(SparkSettings, SeedStarterFlowsResult), String> {
    initialize_runtime_with_options(
        &RuntimeInitializationOptions {
            data_dir: args.data_dir.clone(),
            flows_dir: args.flows_dir.clone(),
            ui_dir,
            force: args.force,
        },
        env,
    )
}

pub fn initialize_runtime_with_options(
    options: &RuntimeInitializationOptions,
    env: &impl Environment,
) -> std::result::Result<(SparkSettings, SeedStarterFlowsResult), String> {
    let settings = resolve_server_settings_with_env(
        &SettingsOverrides {
            data_dir: options.data_dir.clone(),
            flows_dir: options.flows_dir.clone(),
            runs_dir: None,
            ui_dir: options.ui_dir.clone(),
        },
        env,
    )
    .map_err(|error| error.to_string())?;
    validate_settings(&settings).map_err(|error| error.to_string())?;

    let result = seed_starter_flows(&settings.flows_dir, options.force)?;
    spark_storage::seed_default_flow_catalog(&settings.config_dir)
        .map_err(|error| error.to_string())?;
    Ok((settings, result))
}

fn run_service_install(args: &ServiceInstallArgs, env: &impl Environment) -> CommandOutput {
    let systemctl = match require_systemd_user_support(env) {
        Ok(path) => path,
        Err(message) => return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    };
    let init_args = InitArgs {
        data_dir: args.data_dir.clone(),
        flows_dir: args.flows_dir.clone(),
        force: false,
    };
    let (settings, result) = match initialize_runtime_with_ui(&init_args, args.ui_dir.clone(), env)
    {
        Ok(result) => result,
        Err(message) => return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    };

    let unit_path = service_unit_path(env);
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return CommandOutput::stderr(
                EXIT_GENERAL_FAILURE,
                format!("Unable to resolve spark-server executable: {error}\n"),
            )
        }
    };
    let unit = build_service_unit(
        &settings,
        &args.host,
        args.port,
        &current_exe,
        env.get_var("PATH").unwrap_or_default().as_str(),
    );

    if let Some(parent) = unit_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n"));
        }
    }
    if let Err(error) = fs::write(&unit_path, unit) {
        return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n"));
    }

    for invocation in [
        vec!["daemon-reload"],
        vec!["enable", SERVICE_UNIT_NAME],
        vec!["restart", SERVICE_UNIT_NAME],
    ] {
        if let Err(output) = run_systemctl_checked(&systemctl, &invocation) {
            return output;
        }
    }

    CommandOutput::stdout(
        0,
        format!(
            "Installed Spark user service: {}\nService name: {}\nListening on http://{}:{}\nInitialized Spark at {}\nSeeded flows: {}\ncreated={} updated={} skipped={}\n",
            unit_path.display(),
            SERVICE_UNIT_NAME,
            args.host,
            args.port,
            settings.data_dir.display(),
            result.flows_dir.display(),
            result.created.len(),
            result.updated.len(),
            result.skipped.len()
        ),
    )
}

fn run_service_remove(env: &impl Environment) -> CommandOutput {
    let systemctl = match require_systemd_user_support(env) {
        Ok(path) => path,
        Err(message) => return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    };
    let unit_path = service_unit_path(env);

    let _ = run_systemctl_unchecked(&systemctl, &["disable", "--now", SERVICE_UNIT_NAME]);
    if unit_path.exists() {
        if let Err(error) = fs::remove_file(&unit_path) {
            return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n"));
        }
    }
    if let Err(output) = run_systemctl_checked(&systemctl, &["daemon-reload"]) {
        return output;
    }
    let _ = run_systemctl_unchecked(&systemctl, &["reset-failed", SERVICE_UNIT_NAME]);

    CommandOutput::stdout(
        0,
        format!("Removed Spark user service: {}\n", unit_path.display()),
    )
}

fn run_service_status(env: &impl Environment) -> CommandOutput {
    let systemctl = match require_systemd_user_support(env) {
        Ok(path) => path,
        Err(message) => return CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    };
    match run_systemctl_unchecked(
        &systemctl,
        &["--no-pager", "--full", "status", SERVICE_UNIT_NAME],
    ) {
        Ok(output) => output,
        Err(message) => CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{message}\n")),
    }
}

fn require_systemd_user_support(env: &impl Environment) -> std::result::Result<PathBuf, String> {
    if std::env::consts::OS != "linux" {
        return Err(
            "Spark service management currently supports Linux user services via systemd only."
                .to_string(),
        );
    }
    find_on_path("systemctl", env).ok_or_else(|| "systemctl is not available on PATH.".to_string())
}

fn service_unit_path(env: &impl Environment) -> PathBuf {
    let config_home = env
        .get_var("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(|value| expand_tilde_with_env(&value, env))
        .unwrap_or_else(|| {
            env.get_var("HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".config")
        });
    config_home
        .join("systemd")
        .join("user")
        .join(SERVICE_UNIT_NAME)
}

fn build_service_unit(
    settings: &SparkSettings,
    host: &str,
    port: u16,
    binary_path: &Path,
    path_env: &str,
) -> String {
    let provider_env_file = settings.config_dir.join("provider.env");
    let mut service_environment = vec![
        quote_systemd_arg(format!("PATH={path_env}")),
        quote_systemd_arg(format!("SPARK_HOME={}", settings.data_dir.display())),
        quote_systemd_arg(format!("SPARK_FLOWS_DIR={}", settings.flows_dir.display())),
    ];
    if let Some(ui_dir) = settings.ui_dir.as_deref() {
        service_environment.push(quote_systemd_arg(format!(
            "SPARK_UI_DIR={}",
            ui_dir.display()
        )));
    }

    let mut exec_args = vec![
        binary_path.to_string_lossy().into_owned(),
        "serve".to_string(),
        "--host".to_string(),
        host.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--data-dir".to_string(),
        settings.data_dir.to_string_lossy().into_owned(),
        "--flows-dir".to_string(),
        settings.flows_dir.to_string_lossy().into_owned(),
    ];
    if let Some(ui_dir) = settings.ui_dir.as_deref() {
        exec_args.push("--ui-dir".to_string());
        exec_args.push(ui_dir.to_string_lossy().into_owned());
    }
    let exec_start = exec_args
        .into_iter()
        .map(quote_systemd_arg)
        .collect::<Vec<_>>()
        .join(" ");

    let mut lines = vec![
        "[Unit]".to_string(),
        "Description=Spark workspace server".to_string(),
        "After=network.target".to_string(),
        String::new(),
        "[Service]".to_string(),
        "Type=simple".to_string(),
        "Restart=on-failure".to_string(),
        "RestartSec=2".to_string(),
        format!("EnvironmentFile=-{}", provider_env_file.display()),
    ];
    lines.extend(
        service_environment
            .into_iter()
            .map(|entry| format!("Environment={entry}")),
    );
    lines.extend([
        format!("ExecStart={exec_start}"),
        String::new(),
        "[Install]".to_string(),
        "WantedBy=default.target".to_string(),
        String::new(),
    ]);
    lines.join("\n")
}

fn quote_systemd_arg(value: impl AsRef<str>) -> String {
    let escaped = value
        .as_ref()
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    if escaped.is_empty() || escaped.chars().any(char::is_whitespace) {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

fn run_systemctl_checked(
    systemctl: &Path,
    args: &[&str],
) -> std::result::Result<(), CommandOutput> {
    match run_systemctl_unchecked(systemctl, args) {
        Ok(output) if output.exit_code == 0 => Ok(()),
        Ok(output) => {
            let details = output.stderr.trim();
            let stdout_details = output.stdout.trim();
            let message = if !details.is_empty() {
                details.to_string()
            } else if !stdout_details.is_empty() {
                stdout_details.to_string()
            } else {
                format!("systemctl exited with status {}", output.exit_code)
            };
            Err(CommandOutput::stderr(
                EXIT_GENERAL_FAILURE,
                format!("{message}\n"),
            ))
        }
        Err(message) => Err(CommandOutput::stderr(
            EXIT_GENERAL_FAILURE,
            format!("{message}\n"),
        )),
    }
}

fn run_systemctl_unchecked(
    systemctl: &Path,
    args: &[&str],
) -> std::result::Result<CommandOutput, String> {
    let output = Command::new(systemctl)
        .arg("--user")
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    Ok(CommandOutput {
        exit_code: output.status.code().unwrap_or(EXIT_GENERAL_FAILURE),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn find_on_path(name: &str, env: &impl Environment) -> Option<PathBuf> {
    let path = env.get_var("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn expand_tilde_with_env(value: &str, env: &impl Environment) -> PathBuf {
    if value == "~" {
        return env
            .get_var("HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env
            .get_var("HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

pub fn build_serve_configuration_from_args(
    args: &[String],
    env: &impl Environment,
) -> std::result::Result<ServeConfiguration, CommandOutput> {
    let serve_args = parse_serve_args(&args[1..]).map_err(usage_error)?;
    if let Some(guard_output) =
        enforce_runtime_home_guard("spark-server serve", serve_args.data_dir.as_deref(), env)
    {
        return Err(guard_output);
    }
    let settings = resolve_server_settings_with_env(
        &SettingsOverrides {
            data_dir: serve_args.data_dir.clone(),
            flows_dir: serve_args.flows_dir.clone(),
            runs_dir: None,
            ui_dir: serve_args.ui_dir.clone(),
        },
        env,
    )
    .map_err(|error| CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n")))?;
    validate_settings(&settings)
        .map_err(|error| CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n")))?;
    Ok(ServeConfiguration {
        host: serve_args.host,
        port: serve_args.port,
        reload: serve_args.reload,
        debug_codex_jsonrpc: serve_args.debug_codex_jsonrpc
            || codex_jsonrpc_trace_enabled_with_env(env),
        settings,
    })
}

fn resolve_server_settings_with_env(
    overrides: &SettingsOverrides,
    env: &impl Environment,
) -> std::result::Result<SparkSettings, SparkCommonError> {
    let executable_path = std::env::current_exe().ok();
    resolve_server_settings_with_executable_path(overrides, env, executable_path.as_deref())
}

pub fn resolve_server_settings_with_executable_path(
    overrides: &SettingsOverrides,
    env: &impl Environment,
    executable_path: Option<&Path>,
) -> std::result::Result<SparkSettings, SparkCommonError> {
    let mut settings = resolve_settings_with_env(overrides, env)?;
    if let Some(package_root) = executable_path
        .and_then(|path| installed_package_root_from_executable(path, "spark-server"))
    {
        settings.project_root = package_root;
    }
    Ok(settings)
}

fn run_serve_process(
    args: &[String],
    env: &impl Environment,
) -> std::result::Result<(), CommandOutput> {
    let config = build_serve_configuration_from_args(args, env)?;
    if config.debug_codex_jsonrpc {
        std::env::set_var(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    }
    let bind_addr = format!("{}:{}", config.host, config.port);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n")))?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&bind_addr)
            .await
            .map_err(|error| CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n")))?;
        let _ = spark_agent_adapter::configure_codex_api_base_url(format!(
            "http://{}:{}",
            config.host, config.port
        ));
        let client = rust_llm_client_from_settings(&config.settings);
        axum_serve(
            listener,
            spark_http::build_app_with_rust_llm_client(config.settings, client),
        )
        .await
    })
}

fn seed_starter_flows(
    flows_dir: &Path,
    force: bool,
) -> std::result::Result<SeedStarterFlowsResult, String> {
    let assets = spark_assets::flows::starter_flow_assets()
        .map_err(|error| format!("Packaged flow assets are unavailable: {error:?}"))?;
    fs::create_dir_all(flows_dir).map_err(|source| {
        format!(
            "Unable to create flows directory {}: {source}",
            flows_dir.display()
        )
    })?;

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut skipped = Vec::new();

    for asset in assets {
        let relative_name = asset.name;
        let target_path = flows_dir.join(&relative_name);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                format!(
                    "Unable to create flow parent directory {}: {source}",
                    parent.display()
                )
            })?;
        }

        let existed = target_path.exists();
        if existed && !force {
            skipped.push(relative_name);
            continue;
        }

        fs::write(&target_path, asset.content).map_err(|source| {
            format!(
                "Unable to write starter flow {}: {source}",
                target_path.display()
            )
        })?;
        if existed {
            updated.push(relative_name);
        } else {
            created.push(relative_name);
        }
    }

    Ok(SeedStarterFlowsResult {
        flows_dir: flows_dir.to_path_buf(),
        created,
        updated,
        skipped,
    })
}

fn enforce_runtime_home_guard(
    command_name: &'static str,
    data_dir: Option<&Path>,
    env: &impl Environment,
) -> Option<CommandOutput> {
    let project_root = source_guard_root("spark-server");
    match require_explicit_dev_home_with_env(command_name, data_dir, &project_root, env) {
        Ok(()) => None,
        Err(SparkCommonError::SourceCheckoutGuard(message)) => Some(CommandOutput::stderr(
            EXIT_GENERAL_FAILURE,
            format!("{message}\n"),
        )),
        Err(error) => Some(CommandOutput::stderr(
            EXIT_GENERAL_FAILURE,
            format!("{error}\n"),
        )),
    }
}

fn parse_init_args(args: &[String]) -> std::result::Result<InitArgs, String> {
    let mut parsed = InitArgs::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => {
                parsed.data_dir = Some(next_path(args, &mut index, "--data-dir")?);
            }
            "--flows-dir" => {
                parsed.flows_dir = Some(next_path(args, &mut index, "--flows-dir")?);
            }
            "--force" => {
                parsed.force = true;
                index += 1;
            }
            value if is_help_arg(value) => return Ok(parsed),
            unknown => return Err(format!("unrecognized arguments: {unknown}")),
        }
    }
    Ok(parsed)
}

fn parse_serve_args(args: &[String]) -> std::result::Result<ServeArgs, String> {
    let mut parsed = ServeArgs {
        host: "127.0.0.1".to_string(),
        port: 8000,
        ..ServeArgs::default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => parsed.data_dir = Some(next_path(args, &mut index, "--data-dir")?),
            "--flows-dir" => parsed.flows_dir = Some(next_path(args, &mut index, "--flows-dir")?),
            "--ui-dir" => parsed.ui_dir = Some(next_path(args, &mut index, "--ui-dir")?),
            "--host" => parsed.host = next_value(args, &mut index, "--host")?,
            "--port" => {
                let value = next_value(args, &mut index, "--port")?;
                parsed.port = value
                    .parse::<u16>()
                    .map_err(|_| format!("argument --port: invalid int value: '{value}'"))?;
            }
            "--reload" => {
                parsed.reload = true;
                index += 1;
            }
            "--debug-codex-jsonrpc" => {
                parsed.debug_codex_jsonrpc = true;
                index += 1;
            }
            value if is_help_arg(value) => return Ok(parsed),
            unknown => return Err(format!("unrecognized arguments: {unknown}")),
        }
    }
    Ok(parsed)
}

fn parse_service_install_args(args: &[String]) -> std::result::Result<ServiceInstallArgs, String> {
    let mut parsed = ServiceInstallArgs {
        host: "0.0.0.0".to_string(),
        port: 8000,
        ..ServiceInstallArgs::default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => parsed.data_dir = Some(next_path(args, &mut index, "--data-dir")?),
            "--flows-dir" => parsed.flows_dir = Some(next_path(args, &mut index, "--flows-dir")?),
            "--ui-dir" => parsed.ui_dir = Some(next_path(args, &mut index, "--ui-dir")?),
            "--host" => parsed.host = next_value(args, &mut index, "--host")?,
            "--port" => {
                let value = next_value(args, &mut index, "--port")?;
                parsed.port = value
                    .parse::<u16>()
                    .map_err(|_| format!("argument --port: invalid int value: '{value}'"))?;
            }
            value if is_help_arg(value) => return Ok(parsed),
            unknown => return Err(format!("unrecognized arguments: {unknown}")),
        }
    }
    Ok(parsed)
}

fn next_path(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> std::result::Result<PathBuf, String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("argument {flag}: expected one argument"))?;
    *index += 2;
    Ok(PathBuf::from(value))
}

fn next_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> std::result::Result<String, String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("argument {flag}: expected one argument"))?;
    *index += 2;
    Ok(value.to_string())
}

async fn axum_serve(
    listener: tokio::net::TcpListener,
    app: axum::Router,
) -> std::result::Result<(), CommandOutput> {
    axum::serve(listener, app)
        .await
        .map_err(|error| CommandOutput::stderr(EXIT_GENERAL_FAILURE, format!("{error}\n")))
}

fn usage_error(message: impl AsRef<str>) -> CommandOutput {
    CommandOutput::stderr(
        EXIT_USAGE_ERROR,
        format!(
            "usage: spark-server [-h] {{serve,init,service}} ...\n\
spark-server: error: {}\n",
            message.as_ref()
        ),
    )
}

fn strip_program_name<I, S>(args: I, binary_name: &str) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut values = args
        .into_iter()
        .map(|arg| arg.into().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if values
        .first()
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .map(|value| value == binary_name)
        .unwrap_or(false)
    {
        values.remove(0);
    }
    values
}

fn source_guard_root(binary_name: &str) -> PathBuf {
    let Ok(executable_path) = std::env::current_exe() else {
        return source_checkout_root_from_manifest();
    };
    if let Some(package_root) =
        installed_package_root_from_executable(&executable_path, binary_name)
    {
        return package_root;
    }
    source_checkout_root_from_manifest()
}

fn is_help_arg(value: &str) -> bool {
    value == "-h" || value == "--help"
}

fn write_process_output(output: &CommandOutput) {
    use std::io::Write;

    if !output.stdout.is_empty() {
        let _ = std::io::stdout().write_all(output.stdout.as_bytes());
    }
    if !output.stderr.is_empty() {
        let _ = std::io::stderr().write_all(output.stderr.as_bytes());
    }
}
