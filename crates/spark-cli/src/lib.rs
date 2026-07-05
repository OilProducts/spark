#![forbid(unsafe_code)]

//! `spark` binary shell for the Rust rewrite.
//!
//! M5-I01 owns shared command parsing, target resolution, payload loading, local
//! file-only flow commands, and output/error helpers. M5-I02 wires the
//! conversation, run, and server-backed flow commands to the Workspace HTTP
//! surface. M5-I03 wires trigger commands to the real Workspace trigger routes.

mod output;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use attractor_dsl::{format_readable_dot, parse_dot, preview_dot_source};
use clap::{Arg, ArgAction, Command};
pub use output::CommandOutput;
use output::{http_error, json_error, success_json, usage_error};
use serde_json::{json, Map, Value};
use spark_common::logging::init_spark_logging;
use spark_common::paths::{Environment, ProcessEnvironment};
use spark_common::source_checkout::{
    installed_package_root_from_executable, require_explicit_agent_base_url_with_env,
    source_checkout_root_from_manifest,
};
use spark_common::SparkCommonError;
use tracing::Level;

pub const EXIT_GENERAL_FAILURE: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;
pub const EXIT_NOT_FOUND: i32 = 3;
pub const DEFAULT_API_BASE_URL: &str = spark_common::source_checkout::DEFAULT_API_BASE_URL;

const TOP_LEVEL_HELP: &str = concat!(
    "usage: spark [-h] {convo,run,flow,trigger} ...\n",
    "\n",
    "Spark agent CLI\n",
    "\n",
    "positional arguments:\n",
    "  {convo,run,flow,trigger}\n",
    "    convo               Conversation-scoped artifact commands\n",
    "    run                 Direct execution commands\n",
    "    flow                Flow discovery and validation\n",
    "    trigger             Workspace trigger management\n",
    "\n",
    "options:\n",
    "  -h, --help            show this help message and exit\n",
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiRequestPlan {
    pub method: HttpMethod,
    pub base_url: String,
    pub path: String,
    pub body: Option<Value>,
    pub text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedOptions {
    values: BTreeMap<String, String>,
    bools: BTreeSet<String>,
    positionals: Vec<String>,
    seen_values: Vec<String>,
}

impl ParsedOptions {
    fn value(&self, flag: &str) -> Option<&str> {
        self.values.get(flag).map(String::as_str)
    }

    fn bool(&self, flag: &str) -> bool {
        self.bools.contains(flag)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionalMode {
    None,
    Exact(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandDomain {
    Convo,
    Run,
    Flow,
    Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandPath {
    domain: CommandDomain,
}

#[derive(Debug, Clone)]
enum RuntimeStdin {
    Process { cache: Option<String> },
    Injected(String),
}

impl RuntimeStdin {
    fn process() -> Self {
        Self::Process { cache: None }
    }

    fn injected(text: impl Into<String>) -> Self {
        Self::Injected(text.into())
    }

    fn read_to_string(&mut self) -> Result<String, std::io::Error> {
        match self {
            Self::Injected(text) => Ok(text.clone()),
            Self::Process { cache } => {
                if let Some(text) = cache.as_ref() {
                    return Ok(text.clone());
                }
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text)?;
                *cache = Some(text.clone());
                Ok(text)
            }
        }
    }
}

/// Runs the `spark` command and writes process output.
pub fn run() -> i32 {
    let _ = init_spark_logging(Level::INFO);
    let output = run_with_args_and_process_stdin(std::env::args_os(), &ProcessEnvironment);
    output::write_process_output(&output);
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
    run_with_args_env_and_stdin(args, env, "")
}

pub fn run_with_args_env_and_stdin<I, S, E>(
    args: I,
    env: &E,
    stdin_text: impl Into<String>,
) -> CommandOutput
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    E: Environment,
{
    let args = strip_program_name(args, "spark");
    let mut stdin = RuntimeStdin::injected(stdin_text);
    run_agent_shell(&args, env, &mut stdin)
}

pub fn request_plan_with_args_env_and_stdin<I, S, E>(
    args: I,
    env: &E,
    stdin_text: impl Into<String>,
) -> Result<ApiRequestPlan, CommandOutput>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    E: Environment,
{
    let args = strip_program_name(args, "spark");
    let mut stdin = RuntimeStdin::injected(stdin_text);
    build_request_plan(&args, env, &mut stdin)
}

pub fn http_status_exit_code(status_code: u16) -> i32 {
    if status_code == 404 {
        EXIT_NOT_FOUND
    } else {
        EXIT_GENERAL_FAILURE
    }
}

fn run_with_args_and_process_stdin<I, S, E>(args: I, env: &E) -> CommandOutput
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    E: Environment,
{
    let args = strip_program_name(args, "spark");
    let mut stdin = RuntimeStdin::process();
    run_agent_shell(&args, env, &mut stdin)
}

fn run_agent_shell(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> CommandOutput {
    if args.is_empty() || is_help_arg(&args[0]) {
        return CommandOutput::stdout(0, TOP_LEVEL_HELP);
    }

    let command_path = match parse_clap_command_path(args) {
        Ok(command_path) => command_path,
        Err(output) => return output,
    };

    match command_path.domain {
        CommandDomain::Convo | CommandDomain::Run => match build_request_plan(args, env, stdin) {
            Ok(plan) => execute_request_plan(&plan),
            Err(output) => output,
        },
        CommandDomain::Trigger => match build_request_plan(args, env, stdin) {
            Ok(plan) => execute_request_plan(&plan),
            Err(output) => output,
        },
        CommandDomain::Flow => run_flow_domain(args, env, stdin),
    }
}

fn run_flow_domain(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> CommandOutput {
    let Some(command) = args.get(1).map(String::as_str) else {
        return usage_error("Unknown command");
    };

    match command {
        "format" => run_flow_format_command(args, env),
        "validate" if has_option(&args[2..], "--file") => run_flow_validate_file_command(args, env),
        "list" | "describe" | "get" | "validate" => match build_request_plan(args, env, stdin) {
            Ok(plan) => execute_request_plan(&plan),
            Err(output) => output,
        },
        _ => usage_error("Unknown command"),
    }
}

fn build_request_plan(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> Result<ApiRequestPlan, CommandOutput> {
    let command_path = parse_clap_command_path(args)?;
    match command_path.domain {
        CommandDomain::Convo => build_convo_plan(args, env, stdin),
        CommandDomain::Run => build_run_plan(args, env, stdin),
        CommandDomain::Flow => build_flow_plan(args, env),
        CommandDomain::Trigger => build_trigger_plan(args, env, stdin),
    }
}

fn build_convo_plan(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> Result<ApiRequestPlan, CommandOutput> {
    match args.get(1).map(String::as_str) {
        Some("run-request") => {
            let options = parse_api_options(
                &args[2..],
                &[
                    "--conversation",
                    "--flow",
                    "--summary",
                    "--goal",
                    "--goal-file",
                    "--launch-context-json",
                    "--launch-context-file",
                    "--model",
                    "--llm-provider",
                    "--llm-profile",
                    "--reasoning-effort",
                    "--execution-profile",
                    "--base-url",
                ],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--conversation", "--flow", "--summary"])?;
            reject_mutually_exclusive(&options, &["--goal", "--goal-file"])?;
            reject_mutually_exclusive(
                &options,
                &["--launch-context-json", "--launch-context-file"],
            )?;
            let base_url = resolve_base_url(&options, "spark convo run-request", env)?;
            let body = build_flow_payload(&options, "spark convo run-request", stdin)?;
            let conversation = non_empty_value(
                &options,
                "--conversation",
                "Missing required --conversation.",
            )?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Post,
                base_url,
                path: format!(
                    "/workspace/api/conversations/by-handle/{}/flow-run-requests",
                    percent_encode_component(&conversation)
                ),
                body: Some(body),
                text: false,
            })
        }
        _ => Err(usage_error("Unknown command")),
    }
}

fn build_run_plan(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> Result<ApiRequestPlan, CommandOutput> {
    let Some(command) = args.get(1).map(String::as_str) else {
        return Err(usage_error("Unknown command"));
    };

    match command {
        "launch" => {
            let options = parse_api_options(
                &args[2..],
                &[
                    "--flow",
                    "--summary",
                    "--conversation",
                    "--project",
                    "--goal",
                    "--goal-file",
                    "--launch-context-json",
                    "--launch-context-file",
                    "--model",
                    "--llm-provider",
                    "--llm-profile",
                    "--reasoning-effort",
                    "--execution-profile",
                    "--base-url",
                ],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--flow", "--summary"])?;
            reject_mutually_exclusive(&options, &["--goal", "--goal-file"])?;
            reject_mutually_exclusive(
                &options,
                &["--launch-context-json", "--launch-context-file"],
            )?;
            let base_url = resolve_base_url(&options, "spark run launch", env)?;
            let mut body =
                object_from_value(build_flow_payload(&options, "spark run launch", stdin)?);
            let conversation = trimmed_option(&options, "--conversation");
            let project_path = trimmed_option(&options, "--project");
            if conversation.is_none() && project_path.is_none() {
                return Err(json_error(
                    "spark run launch requires --project when --conversation is omitted.",
                    EXIT_GENERAL_FAILURE,
                ));
            }
            if let Some(value) = conversation {
                body.insert("conversation_handle".to_string(), json!(value));
            }
            if let Some(value) = project_path {
                body.insert("project_path".to_string(), json!(value));
            }
            Ok(ApiRequestPlan {
                method: HttpMethod::Post,
                base_url,
                path: "/workspace/api/runs/launch".to_string(),
                body: Some(Value::Object(body)),
                text: false,
            })
        }
        "retry" => {
            let options = parse_api_options(
                &args[2..],
                &["--run", "--conversation", "--base-url"],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--run"])?;
            let base_url = resolve_base_url(&options, "spark run retry", env)?;
            let run_id = non_empty_value(&options, "--run", "Missing required --run id.")?;
            let mut body = Map::new();
            if let Some(conversation) = trimmed_option(&options, "--conversation") {
                body.insert("conversation_handle".to_string(), json!(conversation));
            }
            Ok(ApiRequestPlan {
                method: HttpMethod::Post,
                base_url,
                path: format!(
                    "/workspace/api/runs/{}/retry",
                    percent_encode_component(&run_id)
                ),
                body: Some(Value::Object(body)),
                text: false,
            })
        }
        "continue" => {
            let options = parse_api_options(
                &args[2..],
                &[
                    "--run",
                    "--start-node",
                    "--flow-source-mode",
                    "--flow",
                    "--project",
                    "--conversation",
                    "--model",
                    "--llm-provider",
                    "--llm-profile",
                    "--reasoning-effort",
                    "--base-url",
                ],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--run", "--start-node", "--flow-source-mode"])?;
            let base_url = resolve_base_url(&options, "spark run continue", env)?;
            let run_id = non_empty_value(&options, "--run", "Missing required --run id.")?;
            let start_node =
                non_empty_value(&options, "--start-node", "Missing required --start-node.")?;
            let flow_source_mode = non_empty_value(
                &options,
                "--flow-source-mode",
                "Missing required --flow-source-mode.",
            )?;
            if !matches!(flow_source_mode.as_str(), "snapshot" | "flow_name") {
                return Err(usage_error(format!(
                    "argument --flow-source-mode: invalid choice: '{}' (choose from 'snapshot', 'flow_name')",
                    flow_source_mode
                )));
            }
            let mut body = Map::new();
            body.insert("start_node".to_string(), json!(start_node));
            body.insert(
                "flow_source_mode".to_string(),
                json!(flow_source_mode.clone()),
            );
            if flow_source_mode == "flow_name" {
                if let Some(flow_name) = trimmed_option(&options, "--flow") {
                    body.insert("flow_name".to_string(), json!(flow_name));
                }
            }
            insert_trimmed(&mut body, &options, "--project", "project_path");
            insert_trimmed(&mut body, &options, "--conversation", "conversation_handle");
            insert_trimmed(&mut body, &options, "--model", "model");
            insert_trimmed(&mut body, &options, "--llm-provider", "llm_provider");
            insert_trimmed(&mut body, &options, "--llm-profile", "llm_profile");
            insert_trimmed(
                &mut body,
                &options,
                "--reasoning-effort",
                "reasoning_effort",
            );
            Ok(ApiRequestPlan {
                method: HttpMethod::Post,
                base_url,
                path: format!(
                    "/workspace/api/runs/{}/continue",
                    percent_encode_component(&run_id)
                ),
                body: Some(Value::Object(body)),
                text: false,
            })
        }
        "events" => {
            let options = parse_api_options(
                &args[2..],
                &["--after", "--base-url"],
                &["--json"],
                PositionalMode::Exact(1),
            )?;
            let base_url = resolve_base_url(&options, "spark run events", env)?;
            let run_id = options
                .positionals
                .first()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| json_error("Missing required run_id.", EXIT_GENERAL_FAILURE))?;
            let mut path = format!(
                "/workspace/api/live/events?run_id={}",
                percent_encode_component(&run_id)
            );
            if let Some(after) = trimmed_option(&options, "--after") {
                let sequence = after
                    .parse::<i64>()
                    .map_err(|_| usage_error("argument --after: invalid int value"))?;
                if sequence < 0 {
                    return Err(json_error(
                        "--after must be a non-negative integer.",
                        EXIT_USAGE_ERROR,
                    ));
                }
                write!(
                    &mut path,
                    "&run_sequence={}",
                    percent_encode_component(&sequence.to_string())
                )
                .expect("writing to String cannot fail");
            }
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path,
                body: None,
                text: !options.bool("--json"),
            })
        }
        _ => Err(usage_error("Unknown command")),
    }
}

fn build_flow_plan(
    args: &[String],
    env: &impl Environment,
) -> Result<ApiRequestPlan, CommandOutput> {
    let Some(command) = args.get(1).map(String::as_str) else {
        return Err(usage_error("Unknown command"));
    };
    match command {
        "list" => {
            let options = parse_api_options(
                &args[2..],
                &["--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            let base_url = resolve_base_url(&options, "spark flow list", env)?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: "/workspace/api/flows?surface=agent".to_string(),
                body: None,
                text: options.bool("--text"),
            })
        }
        "describe" => {
            let options = parse_api_options(
                &args[2..],
                &["--flow", "--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            require_values(&options, &["--flow"])?;
            let base_url = resolve_base_url(&options, "spark flow describe", env)?;
            let flow = non_empty_value(&options, "--flow", "Missing required --flow name.")?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: format!(
                    "/workspace/api/flows/{}?surface=agent",
                    percent_encode_component(&flow)
                ),
                body: None,
                text: options.bool("--text"),
            })
        }
        "get" => {
            let options = parse_api_options(
                &args[2..],
                &["--flow", "--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            require_values(&options, &["--flow"])?;
            let base_url = resolve_base_url(&options, "spark flow get", env)?;
            let flow = non_empty_value(&options, "--flow", "Missing required --flow name.")?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: format!(
                    "/workspace/api/flows/{}/raw?surface=agent",
                    percent_encode_component(&flow)
                ),
                body: None,
                text: options.bool("--text"),
            })
        }
        "validate" => {
            let options = parse_api_options(
                &args[2..],
                &["--flow", "--file", "--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            require_one_of(&options, &["--flow", "--file"])?;
            reject_mutually_exclusive(&options, &["--flow", "--file"])?;
            if options.value("--file").is_some() {
                return Err(usage_error("local file validation is not server-backed"));
            }
            let base_url = resolve_base_url(&options, "spark flow validate", env)?;
            let flow = non_empty_value(&options, "--flow", "Missing required --flow name.")?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: format!(
                    "/workspace/api/flows/{}/validate",
                    percent_encode_component(&flow)
                ),
                body: None,
                text: options.bool("--text"),
            })
        }
        _ => Err(usage_error("Unknown command")),
    }
}

fn build_trigger_plan(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
) -> Result<ApiRequestPlan, CommandOutput> {
    let Some(command) = args.get(1).map(String::as_str) else {
        return Err(usage_error("Unknown command"));
    };
    match command {
        "list" => {
            let options = parse_api_options(
                &args[2..],
                &["--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            let base_url = resolve_base_url(&options, "spark trigger list", env)?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: "/workspace/api/triggers".to_string(),
                body: None,
                text: options.bool("--text"),
            })
        }
        "describe" => {
            let options = parse_api_options(
                &args[2..],
                &["--id", "--base-url"],
                &["--text"],
                PositionalMode::None,
            )?;
            require_values(&options, &["--id"])?;
            let base_url = resolve_base_url(&options, "spark trigger describe", env)?;
            let trigger_id = non_empty_value(&options, "--id", "Missing required --id.")?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Get,
                base_url,
                path: format!(
                    "/workspace/api/triggers/{}",
                    percent_encode_component(&trigger_id)
                ),
                body: None,
                text: options.bool("--text"),
            })
        }
        "create" => {
            let options = parse_api_options(
                &args[2..],
                &["--json", "--base-url"],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--json"])?;
            let base_url = resolve_base_url(&options, "spark trigger create", env)?;
            let body = read_required_json_object(
                options.value("--json").unwrap_or_default(),
                "Trigger payload",
                stdin,
            )
            .map_err(|message| json_error(message, EXIT_GENERAL_FAILURE))?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Post,
                base_url,
                path: "/workspace/api/triggers".to_string(),
                body: Some(body),
                text: false,
            })
        }
        "update" => {
            let options = parse_api_options(
                &args[2..],
                &["--id", "--json", "--base-url"],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--id", "--json"])?;
            let base_url = resolve_base_url(&options, "spark trigger update", env)?;
            let trigger_id = non_empty_value(&options, "--id", "Missing required --id.")?;
            let body = read_required_json_object(
                options.value("--json").unwrap_or_default(),
                "Trigger payload",
                stdin,
            )
            .map_err(|message| json_error(message, EXIT_GENERAL_FAILURE))?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Patch,
                base_url,
                path: format!(
                    "/workspace/api/triggers/{}",
                    percent_encode_component(&trigger_id)
                ),
                body: Some(body),
                text: false,
            })
        }
        "delete" => {
            let options = parse_api_options(
                &args[2..],
                &["--id", "--base-url"],
                &[],
                PositionalMode::None,
            )?;
            require_values(&options, &["--id"])?;
            let base_url = resolve_base_url(&options, "spark trigger delete", env)?;
            let trigger_id = non_empty_value(&options, "--id", "Missing required --id.")?;
            Ok(ApiRequestPlan {
                method: HttpMethod::Delete,
                base_url,
                path: format!(
                    "/workspace/api/triggers/{}",
                    percent_encode_component(&trigger_id)
                ),
                body: None,
                text: false,
            })
        }
        _ => Err(usage_error("Unknown command")),
    }
}

fn execute_request_plan(plan: &ApiRequestPlan) -> CommandOutput {
    if is_run_events_path(&plan.path) {
        return execute_sse_request(plan);
    }
    if is_flow_raw_path(&plan.path) {
        return execute_flow_raw_request(plan);
    }

    let payload = match request_json(plan) {
        Ok(payload) => payload,
        Err(output) => return output,
    };
    if plan.text {
        if is_flow_list_path(&plan.path) {
            return CommandOutput::stdout(0, flow_list_text(&payload));
        }
        if is_flow_describe_path(&plan.path) {
            return CommandOutput::stdout(0, describe_flow_text(&payload));
        }
        if is_flow_validate_path(&plan.path) {
            return CommandOutput::stdout(0, validate_flow_text(&payload));
        }
        if is_trigger_list_path(&plan.path) {
            return CommandOutput::stdout(0, trigger_list_text(&payload));
        }
        if is_trigger_describe_path(&plan.path) {
            return CommandOutput::stdout(0, describe_trigger_text(&payload));
        }
    }
    success_json(&payload)
}

fn request_json(plan: &ApiRequestPlan) -> Result<Value, CommandOutput> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return Err(json_error(
                format!("Request failed: {error}"),
                EXIT_GENERAL_FAILURE,
            ))
        }
    };
    let url = workspace_url(&plan.base_url, &plan.path);
    let mut request = match plan.method {
        HttpMethod::Get => client.get(&url),
        HttpMethod::Post => client.post(&url),
        HttpMethod::Patch => client.patch(&url),
        HttpMethod::Delete => client.delete(&url),
    };
    if let Some(body) = &plan.body {
        request = request.json(body);
    }
    let response = request.send().map_err(request_failed)?;
    response_json_payload(response)
}

fn response_json_payload(response: reqwest::blocking::Response) -> Result<Value, CommandOutput> {
    let status_code = response.status().as_u16();
    let is_error = response.status().is_client_error() || response.status().is_server_error();
    let text = response.text().map_err(request_failed)?;
    let payload =
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"detail": text.clone()}));
    if is_error {
        return Err(http_error(
            status_code,
            error_message_from_payload(&payload, &text),
        ));
    }
    Ok(payload)
}

fn execute_flow_raw_request(plan: &ApiRequestPlan) -> CommandOutput {
    let response_text = match request_text(plan) {
        Ok(response_text) => response_text,
        Err(output) => return output,
    };
    if plan.text {
        let mut stdout = response_text;
        if !stdout.ends_with('\n') {
            stdout.push('\n');
        }
        return CommandOutput::stdout(0, stdout);
    }
    let flow_name = flow_name_from_raw_path(&plan.path).unwrap_or_default();
    success_json(&json!({
        "name": flow_name,
        "content": response_text,
    }))
}

fn request_text(plan: &ApiRequestPlan) -> Result<String, CommandOutput> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return Err(json_error(
                format!("Request failed: {error}"),
                EXIT_GENERAL_FAILURE,
            ))
        }
    };
    let url = workspace_url(&plan.base_url, &plan.path);
    let request = match plan.method {
        HttpMethod::Get => client.get(&url),
        HttpMethod::Post => client.post(&url),
        HttpMethod::Patch => client.patch(&url),
        HttpMethod::Delete => client.delete(&url),
    };
    let response = request.send().map_err(request_failed)?;
    let status_code = response.status().as_u16();
    let is_error = response.status().is_client_error() || response.status().is_server_error();
    let text = response.text().map_err(request_failed)?;
    if is_error {
        let payload = serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|_| json!({"detail": text.clone()}));
        return Err(http_error(
            status_code,
            error_message_from_payload(&payload, &text),
        ));
    }
    Ok(text)
}

fn execute_sse_request(plan: &ApiRequestPlan) -> CommandOutput {
    let client = match reqwest::blocking::Client::builder().build() {
        Ok(client) => client,
        Err(error) => return json_error(format!("Request failed: {error}"), EXIT_GENERAL_FAILURE),
    };
    let url = workspace_url(&plan.base_url, &plan.path);
    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(error) => return request_failed(error),
    };
    let status_code = response.status().as_u16();
    let is_error = response.status().is_client_error() || response.status().is_server_error();
    if is_error {
        let text = match response.text() {
            Ok(text) => text,
            Err(error) => return request_failed(error),
        };
        let payload = serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|_| json!({"detail": text.clone()}));
        return http_error(status_code, error_message_from_payload(&payload, &text));
    }

    let mut stdout = String::new();
    let reader = BufReader::new(response);
    for payload in iter_sse_payloads(reader) {
        if plan.text {
            stdout.push_str(&run_event_text(&payload));
        } else {
            stdout
                .push_str(&serde_json::to_string(&payload).expect("serializing JSON cannot fail"));
            stdout.push('\n');
        }
    }
    CommandOutput::stdout(0, stdout)
}

fn iter_sse_payloads(reader: impl BufRead) -> Vec<Value> {
    let mut payloads = Vec::new();
    let mut data_lines = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if line.is_empty() {
            flush_sse_payload(&mut data_lines, &mut payloads);
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start().to_string());
        }
    }
    flush_sse_payload(&mut data_lines, &mut payloads);
    payloads
}

fn flush_sse_payload(data_lines: &mut Vec<String>, payloads: &mut Vec<Value>) {
    if data_lines.is_empty() {
        return;
    }
    let raw = data_lines.join("\n");
    data_lines.clear();
    let Ok(payload) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    if payload.is_object() {
        payloads.push(payload);
    }
}

fn request_failed(error: reqwest::Error) -> CommandOutput {
    json_error(format!("Request failed: {error}"), EXIT_GENERAL_FAILURE)
}

fn error_message_from_payload(payload: &Value, fallback_text: &str) -> String {
    let Some(object) = payload.as_object() else {
        return fallback_error_text(fallback_text);
    };
    if let Some(message) = non_empty_json_string(object.get("error")) {
        return message;
    }
    if let Some(detail) = object.get("detail") {
        match detail {
            Value::Array(entries) => {
                return entries
                    .iter()
                    .map(format_validation_error)
                    .collect::<Vec<_>>()
                    .join("; ");
            }
            Value::String(_) => {
                if let Some(message) = non_empty_json_string(Some(detail)) {
                    return message;
                }
            }
            Value::Object(detail_object) => {
                if let Some(message) = non_empty_json_string(detail_object.get("error")) {
                    return message;
                }
            }
            _ => {}
        }
    }
    fallback_error_text(fallback_text)
}

fn non_empty_json_string(value: Option<&Value>) -> Option<String> {
    let trimmed = value?.as_str()?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn fallback_error_text(fallback_text: &str) -> String {
    fallback_text.trim().to_string()
}

fn format_validation_error(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
    };
    let message = object
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("Invalid request.");
    let path = object
        .get("loc")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    let value = part
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| part.to_string());
                    (value != "body").then_some(value)
                })
                .collect::<Vec<_>>()
                .join(".")
        })
        .filter(|path| !path.is_empty());
    match path {
        Some(path) => format!("{path}: {message}"),
        None => message.to_string(),
    }
}

fn flow_list_text(payload: &Value) -> String {
    let rows = payload.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if rows.is_empty() {
        return "No agent-requestable flows found.\n".to_string();
    }
    let mut text = String::new();
    for row in rows {
        let Some(row) = row.as_object() else {
            continue;
        };
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let title = row
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .trim();
        let description = row
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !title.is_empty() && title != name {
            writeln!(&mut text, "{name}: {title}").expect("writing to String cannot fail");
        } else {
            writeln!(&mut text, "{name}").expect("writing to String cannot fail");
        }
        if !description.is_empty() {
            writeln!(&mut text, "  {description}").expect("writing to String cannot fail");
        }
    }
    text
}

fn describe_flow_text(payload: &Value) -> String {
    let Some(object) = payload.as_object() else {
        let mut text = payload.to_string();
        text.push('\n');
        return text;
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(name)
        .trim();
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let graph_label = object
        .get("graph_label")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let graph_goal = object
        .get("graph_goal")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let launch_policy = object
        .get("effective_launch_policy")
        .and_then(Value::as_str)
        .unwrap_or("disabled");
    let features = object.get("features").and_then(Value::as_object);
    let mut text = String::new();
    writeln!(&mut text, "Name: {name}").expect("writing to String cannot fail");
    writeln!(&mut text, "Title: {title}").expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Description: {}",
        if description.is_empty() {
            "(none)"
        } else {
            description
        }
    )
    .expect("writing to String cannot fail");
    writeln!(&mut text, "Launch Policy: {launch_policy}").expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Graph Label: {}",
        if graph_label.is_empty() {
            "(none)"
        } else {
            graph_label
        }
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Stated Goal: {}",
        if graph_goal.is_empty() {
            "(none)"
        } else {
            graph_goal
        }
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Node Count: {}",
        display_json_value(object.get("node_count"))
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Edge Count: {}",
        display_json_value(object.get("edge_count"))
    )
    .expect("writing to String cannot fail");
    if let Some(features) = features {
        writeln!(
            &mut text,
            "Has Human Gate: {}",
            python_bool(
                features
                    .get("has_human_gate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            )
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut text,
            "Has Manager Loop: {}",
            python_bool(
                features
                    .get("has_manager_loop")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            )
        )
        .expect("writing to String cannot fail");
    }
    text
}

fn run_event_text(envelope: &Value) -> String {
    let event = envelope
        .get("payload")
        .filter(|value| value.is_object())
        .unwrap_or(envelope);
    let sequence = event.get("sequence").and_then(Value::as_i64);
    let event_type = event
        .get("type")
        .or_else(|| envelope.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("event");
    let message = event
        .get("summary")
        .or_else(|| event.get("message"))
        .or_else(|| event.get("msg"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut text = String::new();
    if let Some(sequence) = sequence {
        write!(&mut text, "{sequence} ").expect("writing to String cannot fail");
    }
    text.push_str(event_type);
    if !message.is_empty() {
        write!(&mut text, ": {message}").expect("writing to String cannot fail");
    }
    text.push('\n');
    text
}

fn display_json_value(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "null".to_string();
    };
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn python_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn workspace_url(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn is_run_events_path(path: &str) -> bool {
    path.starts_with("/workspace/api/live/events?")
}

fn is_flow_list_path(path: &str) -> bool {
    path == "/workspace/api/flows?surface=agent"
}

fn is_flow_describe_path(path: &str) -> bool {
    path.starts_with("/workspace/api/flows/")
        && path.ends_with("?surface=agent")
        && !path.contains("/raw?")
}

fn is_flow_raw_path(path: &str) -> bool {
    path.starts_with("/workspace/api/flows/") && path.contains("/raw?")
}

fn is_flow_validate_path(path: &str) -> bool {
    path.starts_with("/workspace/api/flows/") && path.ends_with("/validate")
}

fn is_trigger_list_path(path: &str) -> bool {
    path == "/workspace/api/triggers"
}

fn is_trigger_describe_path(path: &str) -> bool {
    path.starts_with("/workspace/api/triggers/") && !path.contains('?')
}

fn flow_name_from_raw_path(path: &str) -> Option<String> {
    let raw = path
        .strip_prefix("/workspace/api/flows/")?
        .strip_suffix("/raw?surface=agent")?;
    Some(percent_decode_component(raw))
}

fn parse_clap_command_path(args: &[String]) -> Result<CommandPath, CommandOutput> {
    let Some(domain) = args.first().map(String::as_str) else {
        return Err(CommandOutput::stdout(0, TOP_LEVEL_HELP));
    };
    let matches = match spark_command_tree()
        .try_get_matches_from(std::iter::once("spark").chain(args.iter().map(String::as_str)))
    {
        Ok(matches) => matches,
        Err(_error) => return Err(legacy_command_path_error(args)),
    };
    let Some((domain_name, domain_matches)) = matches.subcommand() else {
        return Err(legacy_command_path_error(args));
    };
    match domain_name {
        "convo" => match domain_matches.subcommand_name() {
            Some("run-request") => Ok(CommandPath {
                domain: CommandDomain::Convo,
            }),
            _ => Err(usage_error("Unknown command")),
        },
        "run" => match domain_matches.subcommand_name() {
            Some("launch" | "retry" | "continue" | "events") => Ok(CommandPath {
                domain: CommandDomain::Run,
            }),
            _ => Err(usage_error("Unknown command")),
        },
        "flow" => match domain_matches.subcommand_name() {
            Some("list" | "describe" | "get" | "validate" | "format") => Ok(CommandPath {
                domain: CommandDomain::Flow,
            }),
            _ => Err(usage_error("Unknown command")),
        },
        "trigger" => match domain_matches.subcommand_name() {
            Some("list" | "describe" | "create" | "update" | "delete") => Ok(CommandPath {
                domain: CommandDomain::Trigger,
            }),
            _ => Err(usage_error("Unknown command")),
        },
        _ => Err(usage_error(format!(
            "argument domain: invalid choice: '{}'",
            domain
        ))),
    }
}

fn legacy_command_path_error(args: &[String]) -> CommandOutput {
    let Some(domain) = args.first().map(String::as_str) else {
        return CommandOutput::stdout(0, TOP_LEVEL_HELP);
    };
    match domain {
        "convo" | "run" | "flow" | "trigger" => usage_error("Unknown command"),
        _ => usage_error(format!("argument domain: invalid choice: '{}'", domain)),
    }
}

fn spark_command_tree() -> Command {
    Command::new("spark")
        .disable_help_flag(true)
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("convo")
                .disable_help_flag(true)
                .disable_help_subcommand(true)
                .subcommand(clap_command_leaf("run-request")),
        )
        .subcommand(
            Command::new("run")
                .disable_help_flag(true)
                .disable_help_subcommand(true)
                .subcommand(clap_command_leaf("launch"))
                .subcommand(clap_command_leaf("retry"))
                .subcommand(clap_command_leaf("continue"))
                .subcommand(clap_command_leaf("events")),
        )
        .subcommand(
            Command::new("flow")
                .disable_help_flag(true)
                .disable_help_subcommand(true)
                .subcommand(clap_command_leaf("list"))
                .subcommand(clap_command_leaf("describe"))
                .subcommand(clap_command_leaf("get"))
                .subcommand(clap_command_leaf("validate"))
                .subcommand(clap_command_leaf("format")),
        )
        .subcommand(
            Command::new("trigger")
                .disable_help_flag(true)
                .disable_help_subcommand(true)
                .subcommand(clap_command_leaf("list"))
                .subcommand(clap_command_leaf("describe"))
                .subcommand(clap_command_leaf("create"))
                .subcommand(clap_command_leaf("update"))
                .subcommand(clap_command_leaf("delete")),
        )
}

fn clap_command_leaf(name: &'static str) -> Command {
    Command::new(name)
        .disable_help_flag(true)
        .disable_help_subcommand(true)
        .trailing_var_arg(true)
        .arg(
            Arg::new("argv")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

fn percent_decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    index += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn parse_api_options(
    args: &[String],
    value_flags: &[&'static str],
    bool_flags: &[&'static str],
    positionals: PositionalMode,
) -> Result<ParsedOptions, CommandOutput> {
    parse_options(args, value_flags, bool_flags, positionals).map_err(usage_error)
}

fn parse_options(
    args: &[String],
    value_flags: &[&'static str],
    bool_flags: &[&'static str],
    positionals: PositionalMode,
) -> Result<ParsedOptions, String> {
    scan_options_for_argparse_error(args, value_flags, bool_flags, positionals)?;
    let matches = clap_options_command(value_flags, bool_flags, positionals)
        .try_get_matches_from(std::iter::once("spark").chain(args.iter().map(String::as_str)))
        .map_err(|error| argparse_compatible_parse_error(error))?;

    let mut values = BTreeMap::new();
    for flag in value_flags {
        let id = clap_flag_id(flag);
        if let Some(items) = matches.get_many::<String>(id) {
            let values_for_flag = items.cloned().collect::<Vec<_>>();
            if let Some(value) = values_for_flag.last() {
                values.insert((*flag).to_string(), value.clone());
            }
        }
    }

    let mut bools = BTreeSet::new();
    for flag in bool_flags {
        let id = clap_flag_id(flag);
        if matches.get_flag(id) {
            bools.insert((*flag).to_string());
        }
    }

    let positional_values = match positionals {
        PositionalMode::None => Vec::new(),
        PositionalMode::Exact(count) => {
            let values = if count == 1 {
                matches
                    .get_one::<String>("positional")
                    .map(|value| vec![value.clone()])
                    .unwrap_or_default()
            } else {
                matches
                    .get_many::<String>("positional")
                    .map(|items| items.cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            if values.len() < count {
                return Err("the following arguments are required: run_id".to_string());
            }
            values
        }
    };

    Ok(ParsedOptions {
        values,
        bools,
        positionals: positional_values,
        seen_values: seen_value_flags(args, value_flags),
    })
}

fn clap_options_command(
    value_flags: &[&'static str],
    bool_flags: &[&'static str],
    positionals: PositionalMode,
) -> Command {
    let mut command = Command::new("spark")
        .disable_help_flag(true)
        .disable_help_subcommand(true)
        .no_binary_name(false);
    for flag in value_flags {
        let id = clap_flag_id(flag);
        command = command.arg(
            Arg::new(id)
                .long(id)
                .num_args(0..=1)
                .action(ArgAction::Append)
                .allow_hyphen_values(true)
                .default_missing_value(""),
        );
    }
    for flag in bool_flags {
        let id = clap_flag_id(flag);
        command = command.arg(Arg::new(id).long(id).action(ArgAction::SetTrue));
    }
    if let PositionalMode::Exact(count) = positionals {
        let positional = Arg::new("positional").index(1).allow_hyphen_values(true);
        command = if count == 1 {
            command.arg(positional.action(ArgAction::Set))
        } else {
            command.arg(positional.num_args(0..=count).action(ArgAction::Append))
        };
    }
    command
}

fn clap_flag_id(flag: &'static str) -> &'static str {
    flag.strip_prefix("--").unwrap_or(flag)
}

fn argparse_compatible_parse_error(clap_error: clap::Error) -> String {
    clap_error
        .to_string()
        .lines()
        .find_map(|line| line.strip_prefix("error: ").map(str::to_string))
        .unwrap_or_else(|| clap_error.to_string().trim().to_string())
}

fn scan_options_for_argparse_error(
    args: &[String],
    value_flags: &[&'static str],
    bool_flags: &[&'static str],
    positionals: PositionalMode,
) -> Result<(), String> {
    let mut positional_values = Vec::new();
    let mut unknown = Vec::new();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if let Some((flag, _value)) = arg.split_once('=') {
            if value_flags.contains(&flag) {
                index += 1;
                continue;
            }
        }
        if value_flags.contains(&arg.as_str()) {
            let Some(value) = args.get(index + 1) else {
                return Err(format!("argument {arg}: expected one argument"));
            };
            if value.starts_with("--") {
                return Err(format!("argument {arg}: expected one argument"));
            }
            index += 2;
            continue;
        }
        if bool_flags.contains(&arg.as_str()) {
            index += 1;
            continue;
        }
        if arg.starts_with('-') {
            unknown.push(arg.clone());
            if args
                .get(index + 1)
                .map(|next| !next.starts_with('-'))
                .unwrap_or(false)
            {
                unknown.push(args[index + 1].clone());
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        positional_values.push(arg.clone());
        index += 1;
    }

    let allowed_positionals = match positionals {
        PositionalMode::None => 0,
        PositionalMode::Exact(count) => count,
    };
    if positional_values.len() > allowed_positionals {
        unknown.extend(positional_values[allowed_positionals..].iter().cloned());
    }
    if !unknown.is_empty() {
        return Err(format!("unrecognized arguments: {}", unknown.join(" ")));
    }
    if let PositionalMode::Exact(count) = positionals {
        if positional_values.len() < count {
            return Err("the following arguments are required: run_id".to_string());
        }
    }
    Ok(())
}

fn seen_value_flags(args: &[String], value_flags: &[&'static str]) -> Vec<String> {
    let mut seen_values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some((flag, _value)) = arg.split_once('=') {
            if value_flags.contains(&flag) {
                seen_values.push(flag.to_string());
                index += 1;
                continue;
            }
        }
        if value_flags.contains(&arg.as_str()) {
            seen_values.push(arg.clone());
            index += if args
                .get(index + 1)
                .map(|next| !next.starts_with("--"))
                .unwrap_or(false)
            {
                2
            } else {
                1
            };
            continue;
        }
        index += 1;
    }
    seen_values
}

fn require_values(options: &ParsedOptions, flags: &[&str]) -> Result<(), CommandOutput> {
    let missing = flags
        .iter()
        .filter(|flag| !options.values.contains_key(**flag))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(usage_error(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        )))
    }
}

fn require_one_of(options: &ParsedOptions, flags: &[&str]) -> Result<(), CommandOutput> {
    if flags.iter().any(|flag| options.values.contains_key(*flag)) {
        Ok(())
    } else {
        Err(usage_error(format!(
            "one of the arguments {} is required",
            flags.join(" ")
        )))
    }
}

fn reject_mutually_exclusive(options: &ParsedOptions, flags: &[&str]) -> Result<(), CommandOutput> {
    let seen = options
        .seen_values
        .iter()
        .filter(|flag| flags.contains(&flag.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if seen.len() <= 1 {
        return Ok(());
    }
    let offending = seen.last().expect("at least two seen");
    let previous = &seen[seen.len() - 2];
    Err(usage_error(format!(
        "argument {offending}: not allowed with argument {previous}"
    )))
}

fn resolve_base_url(
    options: &ParsedOptions,
    command_name: &'static str,
    env: &impl Environment,
) -> Result<String, CommandOutput> {
    let base_url = options
        .value("--base-url")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let project_root = source_guard_root("spark");
    match require_explicit_agent_base_url_with_env(command_name, base_url, &project_root, env) {
        Ok(()) => Ok(base_url
            .map(str::to_string)
            .or_else(|| {
                env.get_var("SPARK_API_BASE_URL")
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())),
        Err(SparkCommonError::SourceCheckoutGuard(message)) => {
            Err(json_error(message, EXIT_GENERAL_FAILURE))
        }
        Err(error) => Err(CommandOutput::stderr(
            EXIT_GENERAL_FAILURE,
            format!("{error}\n"),
        )),
    }
}

fn build_flow_payload(
    options: &ParsedOptions,
    source_name: &str,
    stdin: &mut RuntimeStdin,
) -> Result<Value, CommandOutput> {
    let flow_name = trimmed_option(options, "--flow").ok_or_else(|| {
        json_error(
            format!("{source_name} requires a non-empty flow_name."),
            EXIT_GENERAL_FAILURE,
        )
    })?;
    let summary = trimmed_option(options, "--summary").ok_or_else(|| {
        json_error(
            format!("{source_name} requires a non-empty summary."),
            EXIT_GENERAL_FAILURE,
        )
    })?;
    let goal = read_optional_goal(options.value("--goal"), options.value("--goal-file"), stdin)
        .map_err(|message| json_error(message, EXIT_GENERAL_FAILURE))?;
    let launch_context = read_optional_launch_context(
        options.value("--launch-context-json"),
        options.value("--launch-context-file"),
    )
    .map_err(|message| json_error(message, EXIT_GENERAL_FAILURE))?;

    let mut body = Map::new();
    body.insert("flow_name".to_string(), json!(flow_name));
    body.insert("summary".to_string(), json!(summary));
    if let Some(value) = goal {
        body.insert("goal".to_string(), json!(value));
    }
    if let Some(value) = launch_context {
        body.insert("launch_context".to_string(), value);
    }
    insert_trimmed(&mut body, options, "--model", "model");
    if let Some(value) = trimmed_option(options, "--llm-provider") {
        body.insert("llm_provider".to_string(), json!(value.to_lowercase()));
    }
    insert_trimmed(&mut body, options, "--llm-profile", "llm_profile");
    if let Some(value) = trimmed_option(options, "--reasoning-effort") {
        body.insert("reasoning_effort".to_string(), json!(value.to_lowercase()));
    }
    insert_trimmed(
        &mut body,
        options,
        "--execution-profile",
        "execution_profile_id",
    );
    Ok(Value::Object(body))
}

fn run_flow_format_command(args: &[String], env: &impl Environment) -> CommandOutput {
    let options = match parse_flow_file_options(&args[2..], false) {
        Ok(options) => options,
        Err(message) => return usage_error(message),
    };

    let flow_path = expand_user_path(&options.file, env);
    let raw_content = match std::fs::read_to_string(&flow_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json_error(
                format!("Flow file not found: {}", options.file),
                EXIT_GENERAL_FAILURE,
            );
        }
        Err(error) => {
            return json_error(
                format!("Unable to read flow file {}: {error}", options.file),
                EXIT_GENERAL_FAILURE,
            );
        }
    };

    let graph = match parse_dot(&raw_content) {
        Ok(graph) => graph,
        Err(error) => {
            return json_error(format!("invalid DOT: {error}"), EXIT_GENERAL_FAILURE);
        }
    };

    let formatted = format_readable_dot(&graph);
    if options.write {
        if let Err(error) = std::fs::write(&flow_path, &formatted) {
            return json_error(
                format!("Unable to write flow file {}: {error}", options.file),
                EXIT_GENERAL_FAILURE,
            );
        }
        CommandOutput::stdout(0, "")
    } else {
        CommandOutput::stdout(0, formatted)
    }
}

fn run_flow_validate_file_command(args: &[String], env: &impl Environment) -> CommandOutput {
    let options = match parse_flow_file_options(&args[2..], true) {
        Ok(options) => options,
        Err(message) => return usage_error(message),
    };

    let flow_path = expand_user_path(&options.file, env);
    let raw_content = match std::fs::read_to_string(&flow_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json_error(
                format!("Flow file not found: {}", options.file),
                EXIT_GENERAL_FAILURE,
            );
        }
        Err(error) => {
            return json_error(
                format!("Unable to read flow file {}: {error}", options.file),
                EXIT_GENERAL_FAILURE,
            );
        }
    };

    let preview = preview_dot_source(&raw_content);
    let mut response = object_from_value(preview.payload);
    response.insert(
        "name".to_string(),
        json!(flow_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()),
    );
    response.insert("path".to_string(), json!(resolve_display_path(&flow_path)));
    let payload = Value::Object(response);
    if options.text {
        CommandOutput::stdout(0, validate_flow_text(&payload))
    } else {
        success_json(&payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowFileOptions {
    file: String,
    write: bool,
    text: bool,
}

fn parse_flow_file_options(args: &[String], validate: bool) -> Result<FlowFileOptions, String> {
    let value_flags = if validate {
        &["--flow", "--file", "--base-url"][..]
    } else {
        &["--file"][..]
    };
    let options = parse_options(
        args,
        value_flags,
        &["--write", "--text"],
        PositionalMode::None,
    )?;
    if validate {
        require_one_of_for_local(&options, &["--flow", "--file"])?;
        if options.value("--flow").is_some() && options.value("--file").is_some() {
            return Err("argument --file: not allowed with argument --flow".to_string());
        }
        if options.value("--flow").is_some() {
            return Err("local file validation requires --file".to_string());
        }
    } else if !options.values.contains_key("--file") {
        return Err("the following arguments are required: --file".to_string());
    }
    Ok(FlowFileOptions {
        file: options.value("--file").unwrap_or_default().to_string(),
        write: options.bool("--write"),
        text: options.bool("--text"),
    })
}

fn require_one_of_for_local(options: &ParsedOptions, flags: &[&str]) -> Result<(), String> {
    if flags.iter().any(|flag| options.values.contains_key(*flag)) {
        Ok(())
    } else {
        Err(format!(
            "one of the arguments {} is required",
            flags.join(" ")
        ))
    }
}

fn validate_flow_text(payload: &Value) -> String {
    let Some(object) = payload.as_object() else {
        let mut text = payload.to_string();
        text.push('\n');
        return text;
    };
    let diagnostics = object
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let errors = object
        .get("errors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut text = String::new();
    writeln!(
        &mut text,
        "Name: {}",
        object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Path: {}",
        object
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("(unknown)")
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Status: {}",
        object
            .get("status")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("(unknown)")
    )
    .expect("writing to String cannot fail");
    writeln!(&mut text, "Diagnostics: {}", diagnostics.len())
        .expect("writing to String cannot fail");
    writeln!(&mut text, "Errors: {}", errors.len()).expect("writing to String cannot fail");
    for diagnostic in diagnostics {
        let Some(diagnostic) = diagnostic.as_object() else {
            continue;
        };
        let severity = diagnostic
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("info")
            .trim()
            .to_uppercase();
        let rule = diagnostic
            .get("rule_id")
            .or_else(|| diagnostic.get("rule"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let message = diagnostic
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let line_suffix = diagnostic
            .get("line")
            .and_then(Value::as_i64)
            .filter(|line| *line > 0)
            .map(|line| format!(" line {line}"))
            .unwrap_or_default();
        let rule_prefix = if rule.is_empty() {
            String::new()
        } else {
            format!(" {rule}")
        };
        writeln!(
            &mut text,
            "- {severity}{rule_prefix}{line_suffix}: {message}"
        )
        .expect("writing to String cannot fail");
    }
    text
}

fn trigger_list_text(payload: &Value) -> String {
    let rows = payload.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut text = String::new();
    for row in rows {
        let Some(row) = row.as_object() else {
            continue;
        };
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let trigger_id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let source_type = row
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let enabled = row.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        let protected = row
            .get("protected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let flow_name = row
            .get("action")
            .and_then(Value::as_object)
            .and_then(|action| action.get("flow_name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        writeln!(
            &mut text,
            "{trigger_id}: {name} [{source_type}] -> {flow_name}"
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut text,
            "  enabled={} protected={}",
            python_bool(enabled),
            python_bool(protected)
        )
        .expect("writing to String cannot fail");
    }
    text
}

fn describe_trigger_text(payload: &Value) -> String {
    let Some(object) = payload.as_object() else {
        let mut text = payload.to_string();
        text.push('\n');
        return text;
    };
    let action = object.get("action").and_then(Value::as_object);
    let state = object.get("state").and_then(Value::as_object);
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let protected = object
        .get("protected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let project_path = action
        .and_then(|action| action.get("project_path"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("(none)");
    let last_fired_at = state
        .and_then(|state| state.get("last_fired_at"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("(never)");
    let last_result = state
        .and_then(|state| state.get("last_result"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("(none)");
    let next_run_at = state
        .and_then(|state| state.get("next_run_at"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("(n/a)");

    let mut text = String::new();
    writeln!(
        &mut text,
        "ID: {}",
        object.get("id").and_then(Value::as_str).unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Name: {}",
        object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Source Type: {}",
        object
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(&mut text, "Enabled: {}", python_bool(enabled))
        .expect("writing to String cannot fail");
    writeln!(&mut text, "Protected: {}", python_bool(protected))
        .expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Flow Target: {}",
        action
            .and_then(|action| action.get("flow_name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(&mut text, "Project Target: {project_path}").expect("writing to String cannot fail");
    writeln!(&mut text, "Last Fired: {last_fired_at}").expect("writing to String cannot fail");
    writeln!(&mut text, "Last Result: {last_result}").expect("writing to String cannot fail");
    writeln!(&mut text, "Next Run: {next_run_at}").expect("writing to String cannot fail");
    if let Some(webhook_secret) = object
        .get("webhook_secret")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        writeln!(&mut text, "Webhook Secret: {webhook_secret}")
            .expect("writing to String cannot fail");
    }
    text
}

fn read_optional_goal(
    goal_text: Option<&str>,
    goal_file: Option<&str>,
    stdin: &mut RuntimeStdin,
) -> Result<Option<String>, String> {
    if let Some(value) = goal_text.map(str::trim).filter(|value| !value.is_empty()) {
        if value == "-" {
            let text = stdin
                .read_to_string()
                .map_err(|error| format!("Unable to read goal from stdin: {error}"))?
                .trim()
                .to_string();
            return Ok((!text.is_empty()).then_some(text));
        }
        return Ok(Some(value.to_string()));
    }
    if let Some(path) = goal_file.map(str::trim).filter(|value| !value.is_empty()) {
        let text = std::fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("Goal file not found: {path}")
            } else {
                format!("Unable to read goal file {path}: {error}")
            }
        })?;
        let text = text.trim().to_string();
        return Ok((!text.is_empty()).then_some(text));
    }
    Ok(None)
}

fn read_optional_launch_context(
    launch_context_json: Option<&str>,
    launch_context_file: Option<&str>,
) -> Result<Option<Value>, String> {
    let raw = if let Some(value) = launch_context_json
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value.to_string())
    } else if let Some(path) = launch_context_file
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(std::fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("Launch context file not found: {path}")
            } else {
                format!("Unable to read launch context file {path}: {error}")
            }
        })?)
    } else {
        None
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("Launch context must be valid JSON: {error}"))?;
    if !value.is_object() {
        return Err("Launch context must be a JSON object.".to_string());
    }
    Ok(Some(value))
}

fn read_required_json_object(
    path_or_stdin: &str,
    label: &str,
    stdin: &mut RuntimeStdin,
) -> Result<Value, String> {
    let raw_payload = if path_or_stdin == "-" {
        stdin
            .read_to_string()
            .map_err(|error| format!("Unable to read {label} from stdin: {error}"))?
    } else {
        std::fs::read_to_string(path_or_stdin).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("{label} file not found: {path_or_stdin}")
            } else {
                format!("Unable to read {label} file {path_or_stdin}: {error}")
            }
        })?
    };
    let parsed: Value = serde_json::from_str(&raw_payload).map_err(|error| {
        format!(
            "Invalid JSON in {path_or_stdin} at line {}, column {}: {}.",
            error.line(),
            error.column(),
            serde_json_message_without_location(&error)
        )
    })?;
    if !parsed.is_object() {
        return Err(format!("{label} must be a JSON object."));
    }
    Ok(parsed)
}

fn serde_json_message_without_location(error: &serde_json::Error) -> String {
    let mut message = error.to_string();
    let suffix = format!(" at line {} column {}", error.line(), error.column());
    if let Some(stripped) = message.strip_suffix(&suffix) {
        message = stripped.to_string();
    }
    message
}

fn trimmed_option(options: &ParsedOptions, flag: &str) -> Option<String> {
    options
        .value(flag)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn non_empty_value(
    options: &ParsedOptions,
    flag: &str,
    message: &str,
) -> Result<String, CommandOutput> {
    trimmed_option(options, flag).ok_or_else(|| json_error(message, EXIT_GENERAL_FAILURE))
}

fn insert_trimmed(object: &mut Map<String, Value>, options: &ParsedOptions, flag: &str, key: &str) {
    if let Some(value) = trimmed_option(options, flag) {
        object.insert(key.to_string(), json!(value));
    }
}

fn object_from_value(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}

fn expand_user_path(value: &str, env: &impl Environment) -> PathBuf {
    if value.is_empty() {
        return PathBuf::from(".");
    }
    if value == "~" {
        return env
            .get_var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env.get_var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn resolve_display_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            }
        })
        .to_string_lossy()
        .into_owned()
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

fn has_option(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| {
        arg == flag
            || arg
                .strip_prefix(flag)
                .is_some_and(|rest| rest.starts_with('='))
    })
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
