#![forbid(unsafe_code)]

//! The `spark` command. Each leaf of the clap command tree sends one Spark
//! HTTP request, except `flow validate --dir`, which checks a flow directory
//! locally.

mod output;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use attractor_dsl::{FlowDefinition, FlowDefinitionError};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
pub use output::CommandOutput;
use output::{failure, json_error, success_json, usage_error};
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

/// The client's limit on one request, and so on a `run events` stream
/// without `--until-done`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
    Delete,
}

/// The request a command sends, and how its response is printed.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiRequestPlan {
    pub method: HttpMethod,
    pub base_url: String,
    /// Where `base_url` came from: `flag`, `env`, `saved` or `default`.
    pub target_source: &'static str,
    pub path: String,
    pub body: Option<Value>,
    pub view: View,
    pub hint: Option<Hint>,
}

/// How a command prints its response.
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    /// The server payload, pretty-printed.
    Json,
    FlowListText,
    FlowFilesText,
    /// Only the inputs `flow.md` declares.
    FlowInputs,
    TriggerListText,
    TriggerText,
    /// A validation report, exiting 1 when it records an error.
    Validation {
        text: bool,
    },
    /// The run's event stream, one line per event.
    Events {
        run_id: String,
        after: Option<u64>,
        text: bool,
        until_done: bool,
    },
    /// Polls `<path>/wait` until the delegation seals or the timeout passes.
    DelegationWait {
        timeout: Option<Duration>,
    },
    /// Sends the answer once; a lost response is reported, never resent.
    AnswerOnce,
    /// Reads the delegation at `path`, then answers its only open question once.
    AnswerOpenQuestion,
}

/// What a failed request adds to its error to name the next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A 404 for this flow lists the flows whose names end in it, from the
    /// project, else the conversation's project.
    Flow {
        name: String,
        project: Option<String>,
        conversation: Option<String>,
    },
    /// A 409 retry of a run that did not fail names `run continue`.
    Retry { run_id: String },
}

/// Stdin for flags given `-`. Only one flag may read it.
struct RuntimeStdin {
    injected: Option<String>,
    read_by: Option<String>,
}

impl RuntimeStdin {
    fn new(injected: Option<String>) -> Self {
        Self {
            injected,
            read_by: None,
        }
    }

    fn read(&mut self, flag: &str) -> Result<String, CommandOutput> {
        if let Some(other) = self.read_by.replace(flag.to_string()) {
            return Err(usage_error(format!(
                "--{other} and --{flag} both read stdin (-); pass one of them inline or as a file."
            )));
        }
        if let Some(text) = self.injected.take() {
            return Ok(text);
        }
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| {
                json_error(
                    format!("Unable to read --{flag} from stdin: {error}"),
                    EXIT_GENERAL_FAILURE,
                )
            })?;
        Ok(text)
    }
}

/// Runs the `spark` command and writes process output.
pub fn run() -> i32 {
    let _ = init_spark_logging(Level::INFO);
    let args = strip_program_name(std::env::args_os(), "spark");
    let output = run_agent_shell(
        &args,
        &ProcessEnvironment,
        &mut RuntimeStdin::new(None),
        &mut std::io::stdout(),
    );
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
    let mut stdin = RuntimeStdin::new(Some(stdin_text.into()));
    let mut streamed = Vec::new();
    let mut output = run_agent_shell(&args, env, &mut stdin, &mut streamed);
    output
        .stdout
        .insert_str(0, &String::from_utf8_lossy(&streamed));
    output
}

/// The request a command would send, without sending it.
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
    let (names, matches) = parse(&args)?;
    let names = names.iter().map(String::as_str).collect::<Vec<_>>();
    let mut stdin = RuntimeStdin::new(Some(stdin_text.into()));
    match route(&names, &matches, &mut stdin)? {
        Route::Api(plan) => with_target(plan, &names, &matches, env),
        Route::ValidateDir { .. } => Err(json_error(
            "spark flow validate --dir runs locally; it sends no request.",
            EXIT_GENERAL_FAILURE,
        )),
    }
}

pub fn http_status_exit_code(status_code: u16) -> i32 {
    if status_code == 404 {
        EXIT_NOT_FOUND
    } else {
        EXIT_GENERAL_FAILURE
    }
}

fn run_agent_shell(
    args: &[String],
    env: &impl Environment,
    stdin: &mut RuntimeStdin,
    out: &mut dyn Write,
) -> CommandOutput {
    let (names, matches) = match parse(args) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    let names = names.iter().map(String::as_str).collect::<Vec<_>>();
    let plan = match route(&names, &matches, stdin) {
        Ok(Route::Api(plan)) => plan,
        Ok(Route::ValidateDir { dir, project, text }) => {
            return validate_flow_dir(&dir, project.as_deref(), text, env)
        }
        Err(output) => return output,
    };
    match with_target(plan, &names, &matches, env) {
        Ok(plan) => execute(&plan, env, out),
        Err(output) => output,
    }
}

// ---------------------------------------------------------------------------
// The command tree.

fn cli() -> Command {
    Command::new("spark")
        .about("Drive Spark from a shell: flows, runs, messages, missions, roles, triggers, settings and delegation.")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .disable_help_subcommand(true)
        .arg(
            value("base-url", "URL", "Spark server to call; default SPARK_API_BASE_URL, then the saved target")
                .global(true),
        )
        .subcommands([
            flow_command(),
            run_command(),
            send_command(),
            mission_command(),
            role_command(),
            playbook_command(),
            trigger_command(),
            settings_command(),
            environment_command(),
            delegation_command(),
        ])
}

fn flow_command() -> Command {
    domain("flow", "Find, read and validate flows.").subcommands([
        leaf("list", "List the flows this Spark can run.").args([
            project("Include this project's flows"),
            value(
                "environment",
                "ID",
                "Linked environment to list flows on, from spark environment link list",
            ),
            switch("text", "Print a readable list instead of JSON"),
        ]),
        leaf(
            "get",
            "Print a flow's files, or only the inputs it declares.",
        )
        .args([
            primary(
                "NAME",
                "Flow name as spark flow list shows it, e.g. team/work",
            ),
            project("Resolve the flow through this project's definitions"),
            value(
                "environment",
                "ID",
                "Linked environment to read the flow from, from spark environment link list",
            ),
            switch("text", "Print the files as text instead of JSON"),
            switch(
                "inputs",
                "Print only the inputs flow.md declares; read them before launching",
            )
            .conflicts_with("text"),
        ]),
        leaf(
            "validate",
            "Validate a flow by name, or a flow directory on disk.",
        )
        .args([
            Arg::new("id")
                .value_name("NAME")
                .help("Flow name as spark flow list shows it")
                .value_parser(non_blank),
            value("dir", "DIR", "Flow directory to validate locally instead"),
            project("Resolve flows and roles through this project's definitions"),
            switch("text", "Print a readable report instead of JSON"),
        ])
        .group(
            ArgGroup::new("flow-source")
                .args(["id", "dir"])
                .required(true),
        ),
    ])
}

fn run_command() -> Command {
    let run_id = || primary("RUN_ID", "Run id, from spark run list");
    domain("run", "Start, follow and control flow runs.").subcommands([
        leaf(
            "start",
            "Start a flow run from a conversation, or in a project.",
        )
        .args([
            value(
                "conversation",
                "HANDLE",
                "Start from this conversation, in its project and under its mission or role budget",
            ),
            project("Start in this project, with no conversation"),
            value("flow", "NAME", "Flow to run, from spark flow list").required(true),
            text_value("summary", "TEXT", "One line on what this run is for").required(true),
            goal_arg(),
            context_arg(),
            value(
                "node-overrides",
                "JSON",
                "Per-node overrides keyed by node id: inline JSON, a file path, or - for stdin",
            ),
        ])
        .args(model_args())
        .arg(value(
            "execution-profile",
            "ID",
            "Execution profile to run on",
        ))
        .group(
            ArgGroup::new("origin")
                .args(["conversation", "project"])
                .required(true)
                .multiple(true),
        ),
        leaf("list", "List runs.").arg(project("Only this project's runs")),
        leaf("get", "Print a run's status and details.").arg(run_id()),
        leaf("cancel", "Cancel a run.").arg(run_id()),
        leaf("retry", "Retry a failed run from where it failed.").args([
            run_id(),
            value(
                "conversation",
                "HANDLE",
                "Conversation that asks for the retry",
            ),
        ]),
        leaf("continue", "Start a new run from a node of an earlier run.")
            .args([
                run_id(),
                value("from", "NODE", "Node to start from").required(true),
                value(
                    "flow",
                    "NAME",
                    "Run this flow's current definition instead of the run's snapshot",
                ),
                value(
                    "conversation",
                    "HANDLE",
                    "Conversation that asks for the run",
                ),
                project("Project to run in"),
            ])
            .args(model_args()),
        leaf(
            "events",
            "Print a run's events as JSON lines, for 30 seconds or until the run ends.",
        )
        .args([
            run_id(),
            Arg::new("after")
                .long("after")
                .value_name("N")
                .value_parser(clap::value_parser!(u64))
                .help("Only events after this sequence number"),
            switch("text", "Print one readable line per event"),
            switch("until-done", "Keep printing until the run has ended"),
        ]),
        leaf("questions", "List a run's open questions.").arg(run_id()),
        leaf("answer", "Answer a run's open question.")
            .args([
                run_id(),
                value(
                    "question",
                    "QUESTION_ID",
                    "Question id, from spark run questions",
                )
                .required(true),
                value("option", "VALUE", "Value of the option to choose"),
                text_value("text", "TEXT", "Free-text answer"),
                text_value("note", "TEXT", "Note recorded with the answer"),
            ])
            .group(
                ArgGroup::new("answer")
                    .args(["option", "text"])
                    .required(true),
            ),
    ])
}

fn send_command() -> Command {
    leaf("send", "Message another conversation by handle.")
        .arg_required_else_help(true)
        .args([
            value("to", "HANDLE", "Conversation to message").required(true),
            value("from", "HANDLE", "Your own conversation's handle").required(true),
            text_value("message", "TEXT", "Message text").required(true),
        ])
}

fn mission_command() -> Command {
    let mission_id = || primary("MISSION_ID", "Mission id, from spark mission list");
    let project = || project("The mission's project").required(true);
    domain("mission", "Manage a project's missions.").subcommands([
        leaf("list", "List a project's missions.").arg(project()),
        leaf("get", "Print a mission.").args([mission_id(), project()]),
        leaf("create", "Create a mission.")
            .args([
                self::project("The mission's project"),
                value(
                    "conversation",
                    "HANDLE",
                    "Create it from this conversation, in its project",
                ),
                value(
                    "json",
                    "JSON",
                    "Mission fields: inline JSON, a file path, or - for stdin",
                )
                .required(true),
            ])
            .group(
                ArgGroup::new("origin")
                    .args(["project", "conversation"])
                    .required(true),
            ),
        leaf("update", "Change a mission's fields.").args([
            mission_id(),
            project(),
            value(
                "json",
                "JSON",
                "Fields to change: inline JSON, a file path, or - for stdin",
            )
            .required(true),
        ]),
        leaf("start", "Start a mission.").args([mission_id(), project()]),
        leaf("wait", "Mark a mission as waiting.").args([
            mission_id(),
            project(),
            text_value("reason", "TEXT", "What the mission waits for").required(true),
        ]),
        leaf("close", "Close a mission.").args([
            mission_id(),
            project(),
            Arg::new("status")
                .long("status")
                .value_name("STATUS")
                .value_parser(["done", "failed", "canceled"])
                .required(true)
                .help("How the mission ended"),
            text_value("reason", "TEXT", "Why").required(true),
        ]),
    ])
}

fn role_command() -> Command {
    let name = || primary("NAME", "Role name, from spark role list");
    let project = || project("The role's project").required(true);
    domain("role", "Start and stop a project's standing roles.").subcommands([
        leaf("list", "List a project's roles.").arg(project()),
        leaf("start", "Start a role.").args([name(), project()]),
        leaf("stop", "Stop a role.").args([name(), project()]),
    ])
}

fn playbook_command() -> Command {
    domain("playbook", "Find mission playbooks.").subcommands([
        leaf("list", "List playbooks.").arg(project("Include this project's playbooks")),
        leaf("get", "Print a playbook.").args([
            primary("NAME", "Playbook name, from spark playbook list"),
            project("Resolve the playbook through this project's definitions"),
        ]),
    ])
}

fn trigger_command() -> Command {
    let trigger_id = || primary("TRIGGER_ID", "Trigger id, from spark trigger list");
    let project = || project("The trigger's project").required(true);
    let message = || text_value("message", "TEXT", "Why, recorded with the change");
    domain("trigger", "Manage workspace triggers.").subcommands([
        leaf("list", "List triggers.").args([
            self::project("Only this project's triggers"),
            switch("text", "Print a readable list instead of JSON"),
        ]),
        leaf("get", "Print a trigger, with its current revision.").args([
            trigger_id(),
            project(),
            switch("text", "Print a readable summary instead of JSON"),
        ]),
        leaf("create", "Create a trigger.").args([
            project(),
            value("json", "JSON", "Trigger fields: inline JSON, a file path, or - for stdin").required(true),
            message(),
        ]),
        leaf("update", "Change a trigger.").args([
            trigger_id(),
            project(),
            value("json", "JSON", "Fields to change, with the revision from spark trigger get: inline JSON, a file path, or -").required(true),
            message(),
        ]),
        leaf("delete", "Delete a trigger.").args([
            trigger_id(),
            project(),
            value("expected-revision", "REVISION", "The trigger's current revision, from spark trigger get").required(true),
            message(),
        ]),
    ])
}

fn settings_command() -> Command {
    let json = || {
        value(
            "json",
            "JSON",
            "Settings: inline JSON, a file path, or - for stdin",
        )
        .required(true)
    };
    domain("settings", "Read, validate and save workspace settings.").subcommands([
        leaf("get", "Print the effective settings.").args([
            project("Include this project's settings"),
            value(
                "conversation-id",
                "ID",
                "Include this conversation's settings, by conversation id",
            ),
            value("client", "ID", "Include this client's settings"),
        ]),
        leaf("validate", "Check settings without saving them.").arg(json()),
        leaf("set", "Save settings.").arg(json()),
    ])
}

fn environment_command() -> Command {
    domain(
        "environment",
        "This environment's identity, pairing codes, tokens and links to others.",
    )
    .subcommands([
        leaf(
            "show",
            "Print this environment's id, name and protocol version.",
        ),
        leaf(
            "pair-code",
            "Create a code another environment links to this one with.",
        )
        .arg(text_value(
            "label",
            "TEXT",
            "Label for the token the code creates",
        )),
        domain("tokens", "Tokens other environments reach this one with.").subcommands([
            leaf("list", "List tokens."),
            leaf("revoke", "Revoke a token.").arg(primary(
                "TOKEN_ID",
                "Token id, from spark environment tokens list",
            )),
        ]),
        domain("link", "Links from this environment to others.").subcommands([
            leaf("add", "Link to another environment with its pairing code.").args([
                value("origin", "URL", "The other environment's address").required(true),
                value("expected-id", "ID", "The other environment's id").required(true),
                value(
                    "code",
                    "CODE",
                    "Pairing code from spark environment pair-code there",
                )
                .required(true),
                text_value("label", "TEXT", "Label for the link"),
            ]),
            leaf("list", "List linked environments."),
            leaf(
                "projects",
                "List projects registered on a linked environment.",
            )
            .arg(primary(
                "ENVIRONMENT_ID",
                "Linked environment id, from spark environment link list",
            )),
            leaf("remove", "Remove a link.").arg(primary(
                "ENVIRONMENT_ID",
                "Linked environment id, from spark environment link list",
            )),
        ]),
    ])
}

fn delegation_command() -> Command {
    let delegation_id = || {
        primary(
            "DELEGATION_ID",
            "Delegation id, printed by spark delegation start",
        )
    };
    domain("delegation", "Hand a whole task to a linked environment.").subcommands([
        leaf(
            "start",
            "Start a flow run or a conversation in a linked environment's project.",
        )
        .args([
            value(
                "environment",
                "ID",
                "Linked environment, from spark environment link list",
            )
            .required(true),
            value("target-project", "PATH", "Project path in that environment").required(true),
            Arg::new("kind")
                .long("kind")
                .value_name("KIND")
                .value_parser(["flow", "conversation"])
                .required(true)
                .help("Run a flow, or start a conversation with a message"),
            value(
                "request-key",
                "KEY",
                "Your key for this task; resending it never starts a second task",
            )
            .required(true),
            value("run", "RUN_ID", "Your run the task belongs to").conflicts_with("conversation"),
            value(
                "conversation",
                "HANDLE",
                "Your conversation the task belongs to",
            ),
            value("flow", "NAME", "Flow to run, for --kind flow")
                .required_if_eq("kind", "flow")
                .conflicts_with("message"),
            text_value(
                "summary",
                "TEXT",
                "One line on what the run is for, for --kind flow",
            )
            .required_if_eq("kind", "flow")
            .conflicts_with("message"),
            goal_arg().conflicts_with("message"),
            context_arg().conflicts_with("message"),
            text_value("message", "TEXT", "First message, for --kind conversation")
                .required_if_eq("kind", "conversation"),
        ]),
        leaf("list", "List outgoing delegations.").args([
            value(
                "environment",
                "ID",
                "Only delegations to this linked environment",
            ),
            Arg::new("phase")
                .long("phase")
                .value_name("PHASE")
                .value_parser(["created", "accepted", "sealed"])
                .help("Only delegations in this phase"),
            switch("open", "Only unsealed delegations"),
            switch("paused", "Only paused delegations"),
        ]),
        leaf(
            "get",
            "Print a delegation's status, open questions and result.",
        )
        .arg(delegation_id()),
        leaf("wait", "Wait until a delegation seals, then print it.").args([
            delegation_id(),
            Arg::new("timeout")
                .long("timeout")
                .value_name("SECONDS")
                .value_parser(seconds)
                .help("Stop waiting after this many seconds"),
        ]),
        leaf("cancel", "Cancel a delegation.").arg(delegation_id()),
        leaf("answer", "Answer a delegation's open question.")
            .args([
                delegation_id(),
                value(
                    "question",
                    "QUESTION_ID",
                    "Question id, from spark delegation get; optional with --answers or --form",
                )
                .required_unless_present_any(["answers", "form"]),
                value("option", "VALUE", "Value of the option to choose").requires("question"),
                text_value("text", "TEXT", "Free-text answer").requires("question"),
                value(
                    "answers",
                    "JSON",
                    "Answers keyed by question id: inline JSON, a file path, or - for stdin",
                ),
                value(
                    "form",
                    "JSON",
                    "A form question's reply, such as {\"action\":\"accept\",\"content\":{...}} or {\"action\":\"decline\"}: inline JSON, a file path, or - for stdin",
                ),
            ])
            .group(
                ArgGroup::new("answer")
                    .args(["option", "text", "answers", "form"])
                    .required(true),
            ),
    ])
}

fn domain(name: &'static str, about: &'static str) -> Command {
    Command::new(name)
        .about(about)
        .subcommand_required(true)
        .arg_required_else_help(true)
}

fn leaf(name: &'static str, about: &'static str) -> Command {
    Command::new(name).about(about)
}

/// The object a domain is named after: its leaf's first positional.
fn primary(name: &'static str, help: &'static str) -> Arg {
    Arg::new("id")
        .value_name(name)
        .required(true)
        .help(help)
        .value_parser(non_blank)
}

fn value(id: &'static str, name: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(id)
        .value_name(name)
        .help(help)
        .value_parser(non_blank)
}

/// A free-text value, which may start with `-`.
fn text_value(id: &'static str, name: &'static str, help: &'static str) -> Arg {
    value(id, name, help).allow_hyphen_values(true)
}

fn switch(id: &'static str, help: &'static str) -> Arg {
    Arg::new(id).long(id).action(ArgAction::SetTrue).help(help)
}

fn project(help: &'static str) -> Arg {
    value("project", "PATH", help)
}

fn goal_arg() -> Arg {
    text_value("goal", "TEXT", "The run's goal; - reads it from stdin")
}

fn context_arg() -> Arg {
    value(
        "context",
        "JSON",
        "Launch context: inline JSON, a file path, or - for stdin",
    )
}

fn model_args() -> [Arg; 4] {
    [
        value("model", "MODEL", "Model for agent nodes"),
        value("provider", "PROVIDER", "LLM provider, e.g. codex"),
        value("profile", "PROFILE", "LLM profile"),
        value("reasoning-effort", "EFFORT", "Reasoning effort, e.g. high"),
    ]
}

fn non_blank(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("must not be empty".to_string());
    }
    Ok(value.to_string())
}

fn seconds(value: &str) -> Result<Duration, String> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .ok_or_else(|| "expected a number of seconds".to_string())
}

/// The matched command path (`["run", "start"]`) and the leaf's matches.
fn parse(args: &[String]) -> Result<(Vec<String>, ArgMatches), CommandOutput> {
    let mut command = cli();
    let argv = std::iter::once("spark".to_string()).chain(args.iter().cloned());
    let mut matches = command
        .try_get_matches_from_mut(argv)
        .map_err(|error| clap_failure(error, &command, args))?;
    let mut names = Vec::new();
    while let Some((name, sub)) = matches.remove_subcommand() {
        names.push(name);
        matches = sub;
    }
    Ok((names, matches))
}

/// Help on stdout with exit 0; anything else is one JSON usage error that
/// names the next step.
fn clap_failure(error: clap::Error, root: &Command, args: &[String]) -> CommandOutput {
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        return CommandOutput::stdout(0, error.render().to_string());
    }
    // The deepest command the arguments name, skipping option values such
    // as the global `--base-url URL`.
    let mut command = root;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if let Some(long) = word.strip_prefix("--") {
            let takes_value = !long.contains('=')
                && [root, command]
                    .iter()
                    .flat_map(|command| command.get_arguments())
                    .any(|arg| arg.get_long() == Some(long) && arg.get_action().takes_values());
            if takes_value {
                words.next();
            }
        } else if !word.starts_with('-') {
            match command.find_subcommand(word) {
                Some(sub) => command = sub,
                None => break,
            }
        }
    }
    let rendered = error.render().to_string();
    // Drop clap's pointers that name no useful next step for a stray flag.
    let rendered = rendered
        .replace("For more information, try '--help'.", "")
        .lines()
        .filter(|line| !line.contains("as a value, use '-- "))
        .collect::<Vec<_>>()
        .join("\n");
    let (message, usage) = rendered.split_once("\nUsage:").unwrap_or((&rendered, ""));
    let mut message = message
        .trim()
        .trim_start_matches("error: ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if error.kind() == ErrorKind::InvalidSubcommand {
        let valid = command
            .get_subcommands()
            .map(Command::get_name)
            .collect::<Vec<_>>();
        let _ = write!(message, "; valid subcommands: {}", valid.join(", "));
    }
    if let Some(ContextValue::Strings(missing)) = error.get(ContextKind::InvalidArg) {
        for arg in command.get_positionals() {
            let shown = format!("<{}>", arg.get_value_names().unwrap_or_default()[0]);
            if let (true, Some(help)) = (missing.contains(&shown), arg.get_help()) {
                let _ = write!(message, "; {shown}: {help}");
            }
        }
    }
    let usage = match usage.split("\n\n").next().unwrap_or_default().trim() {
        "" => command.clone().render_usage().to_string(),
        usage => format!("Usage: {usage}"),
    };
    usage_error(format!("{message}. {}", usage.trim()))
}

// ---------------------------------------------------------------------------
// From matches to a request.

enum Route {
    Api(ApiRequestPlan),
    ValidateDir {
        dir: String,
        project: Option<String>,
        text: bool,
    },
}

fn request(method: HttpMethod, path: String) -> ApiRequestPlan {
    ApiRequestPlan {
        method,
        base_url: String::new(),
        target_source: "",
        path,
        body: None,
        view: View::Json,
        hint: None,
    }
}

/// Everything that needs no server, JSON inputs included, before a target
/// is resolved.
fn route(names: &[&str], m: &ArgMatches, stdin: &mut RuntimeStdin) -> Result<Route, CommandOutput> {
    use HttpMethod::{Delete, Get, Patch, Post};
    let id = value_of(m, "id").unwrap_or_default();
    let enc_id = encode(&id);
    let project = value_of(m, "project");
    let in_project = query(&[("project_path", project.clone())]);
    let text = switch_of(m, "text");
    let flow_hint = |name: String| {
        Some(Hint::Flow {
            name,
            project: project.clone(),
            conversation: value_of(m, "conversation"),
        })
    };
    let body = |body: Map<String, Value>| Some(Value::Object(body));
    let plan = match names {
        ["flow", "list"] => {
            let path = match value_of(m, "environment") {
                Some(environment) => format!(
                    "/workspace/api/environments/links/{}/flows{in_project}",
                    encode(&environment)
                ),
                None => format!("/workspace/api/flows{in_project}"),
            };
            ApiRequestPlan {
                view: if text { View::FlowListText } else { View::Json },
                ..request(Get, path)
            }
        }
        ["flow", "get"] => {
            let environment = value_of(m, "environment");
            let path = match environment.as_deref() {
                Some(environment) => format!(
                    "/workspace/api/environments/links/{}/flows/{enc_id}/files{in_project}",
                    encode(environment)
                ),
                None => format!("/workspace/api/flows/{enc_id}/files{in_project}"),
            };
            ApiRequestPlan {
                view: if switch_of(m, "inputs") {
                    View::FlowInputs
                } else if text {
                    View::FlowFilesText
                } else {
                    View::Json
                },
                hint: environment
                    .is_none()
                    .then(|| flow_hint(id.clone()))
                    .flatten(),
                ..request(Get, path)
            }
        }
        ["flow", "validate"] => match value_of(m, "dir") {
            Some(dir) => return Ok(Route::ValidateDir { dir, project, text }),
            None => ApiRequestPlan {
                view: View::Validation { text },
                hint: flow_hint(id.clone()),
                ..request(
                    Get,
                    format!("/workspace/api/flows/{enc_id}/validate{in_project}"),
                )
            },
        },
        ["run", "start"] => {
            let conversation = value_of(m, "conversation");
            if conversation.is_some() && project.is_some() {
                return Err(usage_error(
                    "spark run start takes --conversation or --project, not both: a run started from a conversation runs in that conversation's project, under its mission or role budget. Drop --project.",
                ));
            }
            let mut launch = launch_body(m, stdin)?;
            let path = match conversation {
                Some(handle) => format!(
                    "/workspace/api/conversations/by-handle/{}/flow-run-requests",
                    encode(&handle)
                ),
                None => {
                    put(&mut launch, "project_path", project.clone());
                    "/workspace/api/runs/launch".to_string()
                }
            };
            ApiRequestPlan {
                body: body(launch),
                hint: flow_hint(value_of(m, "flow").unwrap_or_default()),
                ..request(Post, path)
            }
        }
        ["run", "list"] => request(Get, format!("/attractor/runs{in_project}")),
        ["run", "get"] => request(Get, format!("/attractor/pipelines/{enc_id}")),
        ["run", "cancel"] => request(Post, format!("/attractor/pipelines/{enc_id}/cancel")),
        ["run", "retry"] => {
            let mut retry = Map::new();
            put(
                &mut retry,
                "conversation_handle",
                value_of(m, "conversation"),
            );
            ApiRequestPlan {
                body: body(retry),
                hint: Some(Hint::Retry { run_id: id.clone() }),
                ..request(Post, format!("/workspace/api/runs/{enc_id}/retry"))
            }
        }
        ["run", "continue"] => {
            let flow = value_of(m, "flow");
            let mode = if flow.is_some() {
                "flow_name"
            } else {
                "snapshot"
            };
            let mut resume = Map::new();
            put(&mut resume, "start_node", value_of(m, "from"));
            put(&mut resume, "flow_source_mode", Some(mode.to_string()));
            put(&mut resume, "flow_name", flow.clone());
            put(&mut resume, "project_path", project.clone());
            put(
                &mut resume,
                "conversation_handle",
                value_of(m, "conversation"),
            );
            model_fields(&mut resume, m);
            ApiRequestPlan {
                body: body(resume),
                hint: flow.and_then(flow_hint),
                ..request(Post, format!("/workspace/api/runs/{enc_id}/continue"))
            }
        }
        ["run", "events"] => {
            let after = m.try_get_one::<u64>("after").ok().flatten().copied();
            let cursor = after
                .map(|after| format!("&run_sequence={after}"))
                .unwrap_or_default();
            ApiRequestPlan {
                view: View::Events {
                    run_id: id.clone(),
                    after,
                    text,
                    until_done: switch_of(m, "until-done"),
                },
                ..request(
                    Get,
                    format!("/workspace/api/live/events?run_id={enc_id}{cursor}"),
                )
            }
        }
        ["run", "questions"] => request(Get, format!("/attractor/pipelines/{enc_id}/questions")),
        ["run", "answer"] => {
            let mut answer = Map::new();
            put(
                &mut answer,
                "selected_value",
                value_of(m, "option").or_else(|| value_of(m, "text")),
            );
            put(&mut answer, "note", value_of(m, "note"));
            let question = encode(&value_of(m, "question").unwrap_or_default());
            ApiRequestPlan {
                body: body(answer),
                ..request(
                    Post,
                    format!("/attractor/pipelines/{enc_id}/questions/{question}/answer"),
                )
            }
        }
        ["send"] => ApiRequestPlan {
            body: Some(json!({
                "conversation_handle": value_of(m, "from"),
                "message": value_of(m, "message"),
            })),
            ..request(
                Post,
                format!(
                    "/workspace/api/conversations/by-handle/{}/messages",
                    encode(&value_of(m, "to").unwrap_or_default())
                ),
            )
        },
        ["mission", leaf] => {
            let mut path = "/workspace/api/missions".to_string();
            if !matches!(*leaf, "list" | "create") {
                path = format!("{path}/{enc_id}");
            }
            if matches!(*leaf, "start" | "wait" | "close") {
                path = format!("{path}/{leaf}");
            }
            path.push_str(&query(&[
                ("conversation_handle", value_of(m, "conversation")),
                ("project_path", project.clone()),
            ]));
            let mission = match *leaf {
                "create" | "update" => {
                    let mut fields = json_input(m, "json", stdin)?.unwrap_or_default();
                    fields.entry("actor").or_insert(json!("assistant"));
                    body(fields)
                }
                "start" => Some(json!({})),
                "wait" => Some(json!({"reason": value_of(m, "reason"), "actor": "assistant"})),
                "close" => Some(json!({
                    "status": value_of(m, "status"),
                    "reason": value_of(m, "reason"),
                    "actor": "assistant",
                })),
                _ => None,
            };
            let method = match *leaf {
                "list" | "get" => Get,
                "update" => Patch,
                _ => Post,
            };
            ApiRequestPlan {
                body: mission,
                ..request(method, path)
            }
        }
        ["role", "list"] => request(Get, format!("/workspace/api/roles{in_project}")),
        ["role", verb] => ApiRequestPlan {
            body: Some(json!({})),
            ..request(
                Post,
                format!("/workspace/api/roles/{enc_id}/{verb}{in_project}"),
            )
        },
        ["playbook", "list"] => request(Get, format!("/workspace/api/playbooks{in_project}")),
        ["playbook", "get"] => request(
            Get,
            format!("/workspace/api/playbooks/{enc_id}{in_project}"),
        ),
        ["trigger", "list"] => ApiRequestPlan {
            view: if text {
                View::TriggerListText
            } else {
                View::Json
            },
            ..request(Get, format!("/workspace/api/triggers{in_project}"))
        },
        ["trigger", "get"] => ApiRequestPlan {
            view: if text { View::TriggerText } else { View::Json },
            ..request(Get, format!("/workspace/api/triggers/{enc_id}{in_project}"))
        },
        ["trigger", "create"] => {
            let mut fields = json_input(m, "json", stdin)?.unwrap_or_default();
            put(&mut fields, "project_path", project.clone());
            put(&mut fields, "message", value_of(m, "message"));
            ApiRequestPlan {
                body: body(fields),
                ..request(Post, "/workspace/api/triggers".to_string())
            }
        }
        ["trigger", "update"] => {
            let mut fields = json_input(m, "json", stdin)?.unwrap_or_default();
            put(&mut fields, "message", value_of(m, "message"));
            ApiRequestPlan {
                body: body(fields),
                ..request(
                    Patch,
                    format!("/workspace/api/triggers/{enc_id}{in_project}"),
                )
            }
        }
        ["trigger", "delete"] => {
            let query = query(&[
                ("project_path", project.clone()),
                ("expected_revision", value_of(m, "expected-revision")),
                ("message", value_of(m, "message")),
            ]);
            request(Delete, format!("/workspace/api/triggers/{enc_id}{query}"))
        }
        ["settings", "get"] => {
            let query = query(&[
                ("project_path", project.clone()),
                ("conversation_id", value_of(m, "conversation-id")),
                ("client_id", value_of(m, "client")),
            ]);
            request(Get, format!("/workspace/api/settings{query}"))
        }
        ["settings", leaf] => {
            let settings = body(json_input(m, "json", stdin)?.unwrap_or_default());
            ApiRequestPlan {
                body: settings,
                ..match *leaf {
                    "set" => request(Patch, "/workspace/api/settings".to_string()),
                    _ => request(Post, "/workspace/api/settings/validate".to_string()),
                }
            }
        }
        ["environment", "show"] => request(Get, "/environment".to_string()),
        ["environment", "pair-code"] => {
            let mut code = Map::new();
            put(&mut code, "label", value_of(m, "label"));
            ApiRequestPlan {
                body: body(code),
                ..request(Post, "/workspace/api/environment/pairing-codes".to_string())
            }
        }
        ["environment", "tokens", "list"] => {
            request(Get, "/workspace/api/environment/tokens".to_string())
        }
        ["environment", "tokens", "revoke"] => request(
            Post,
            format!("/workspace/api/environment/tokens/{enc_id}/revoke"),
        ),
        ["environment", "link", "add"] => {
            let mut link = Map::new();
            for (flag, key) in [
                ("origin", "origin"),
                ("expected-id", "expected_id"),
                ("code", "code"),
                ("label", "label"),
            ] {
                put(&mut link, key, value_of(m, flag));
            }
            ApiRequestPlan {
                body: body(link),
                ..request(Post, "/workspace/api/environments/links".to_string())
            }
        }
        ["environment", "link", "list"] => {
            request(Get, "/workspace/api/environments/links".to_string())
        }
        ["environment", "link", "projects"] => request(
            Get,
            format!("/workspace/api/environments/links/{enc_id}/projects"),
        ),
        ["environment", "link", "remove"] => request(
            Delete,
            format!("/workspace/api/environments/links/{enc_id}"),
        ),
        ["delegation", "list"] => {
            let q = query(&[
                ("environment_id", value_of(m, "environment")),
                ("phase", value_of(m, "phase")),
                ("open", switch_of(m, "open").then(|| "true".to_string())),
                ("paused", switch_of(m, "paused").then(|| "true".to_string())),
            ]);
            request(Get, format!("/workspace/api/delegations{q}"))
        }
        ["delegation", "start"] => {
            let mut task = match value_of(m, "kind").as_deref() {
                Some("flow") => launch_body(m, stdin)?,
                _ => Map::new(),
            };
            for (flag, key) in [
                ("message", "message"),
                ("kind", "kind"),
                ("environment", "environment_id"),
                ("target-project", "project_path"),
                ("request-key", "request_key"),
                ("run", "run_id"),
                ("conversation", "conversation_handle"),
            ] {
                put(&mut task, key, value_of(m, flag));
            }
            ApiRequestPlan {
                body: body(task),
                ..request(Post, "/workspace/api/delegations".to_string())
            }
        }
        ["delegation", leaf] => {
            let path = format!("/workspace/api/delegations/{enc_id}");
            match *leaf {
                "get" => request(Get, path),
                "cancel" => request(Post, format!("{path}/cancel")),
                "wait" => ApiRequestPlan {
                    view: View::DelegationWait {
                        timeout: m.try_get_one::<Duration>("timeout").ok().flatten().copied(),
                    },
                    ..request(Get, path)
                },
                _ => {
                    let answer = match (
                        json_input(m, "answers", stdin)?,
                        json_input(m, "form", stdin)?,
                    ) {
                        (Some(answers), _) => json!({ "answers": answers }),
                        (None, Some(form)) => json!({ "form": form }),
                        (None, None) => json!({
                            "selected_value": value_of(m, "option").or_else(|| value_of(m, "text")),
                        }),
                    };
                    match value_of(m, "question") {
                        Some(question) => ApiRequestPlan {
                            body: Some(answer),
                            view: View::AnswerOnce,
                            ..request(
                                Post,
                                format!("{path}/questions/{}/answer", encode(&question)),
                            )
                        },
                        None => ApiRequestPlan {
                            body: Some(answer),
                            view: View::AnswerOpenQuestion,
                            ..request(Post, path)
                        },
                    }
                }
            }
        }
        _ => {
            return Err(usage_error(format!(
                "Unknown command: spark {}",
                names.join(" ")
            )))
        }
    };
    Ok(Route::Api(plan))
}

/// The launch payload `run start` and `delegation start --kind flow` share.
fn launch_body(
    m: &ArgMatches,
    stdin: &mut RuntimeStdin,
) -> Result<Map<String, Value>, CommandOutput> {
    let mut body = Map::new();
    put(&mut body, "flow_name", value_of(m, "flow"));
    put(&mut body, "summary", value_of(m, "summary"));
    let goal = match value_of(m, "goal") {
        Some(goal) if goal == "-" => Some(stdin.read("goal")?.trim().to_string()),
        goal => goal,
    };
    put(&mut body, "goal", goal.filter(|goal| !goal.is_empty()));
    if let Some(context) = json_input(m, "context", stdin)? {
        body.insert("launch_context".to_string(), Value::Object(context));
    }
    // The server checks each override's fields.
    if let Some(overrides) = json_input(m, "node-overrides", stdin)? {
        body.insert("node_overrides".to_string(), Value::Object(overrides));
    }
    model_fields(&mut body, m);
    put(
        &mut body,
        "execution_profile_id",
        value_of(m, "execution-profile"),
    );
    Ok(body)
}

fn model_fields(body: &mut Map<String, Value>, m: &ArgMatches) {
    put(body, "model", value_of(m, "model"));
    put(
        body,
        "llm_provider",
        value_of(m, "provider").map(|provider| provider.to_lowercase()),
    );
    put(body, "llm_profile", value_of(m, "profile"));
    put(body, "reasoning_effort", value_of(m, "reasoning-effort"));
}

/// A JSON-object flag: inline JSON when the value starts with `{` or `[`,
/// stdin for `-`, else a file path.
fn json_input(
    m: &ArgMatches,
    flag: &str,
    stdin: &mut RuntimeStdin,
) -> Result<Option<Map<String, Value>>, CommandOutput> {
    let Some(raw) = value_of(m, flag) else {
        return Ok(None);
    };
    let fail = |message: String| json_error(message, EXIT_GENERAL_FAILURE);
    let (source, text) = if raw.starts_with(['{', '[']) {
        ("inline value".to_string(), raw.clone())
    } else if raw == "-" {
        ("stdin".to_string(), stdin.read(flag)?)
    } else {
        let text = std::fs::read_to_string(&raw).map_err(|error| {
            fail(if error.kind() == std::io::ErrorKind::NotFound {
                format!("--{flag} file not found: {raw}; pass inline JSON, a file path, or - for stdin.")
            } else {
                format!("Unable to read --{flag} file {raw}: {error}")
            })
        })?;
        (raw.clone(), text)
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => Ok(Some(object)),
        Ok(_) => Err(fail(format!(
            "--{flag} must be a JSON object{}.",
            match flag {
                "node-overrides" => " keyed by node id",
                "answers" => " of question ID to answer",
                _ => "",
            }
        ))),
        Err(error) => Err(fail(format!(
            "--{flag} {source} is not valid JSON: {error}"
        ))),
    }
}

fn value_of(m: &ArgMatches, id: &str) -> Option<String> {
    m.try_get_one::<String>(id).ok().flatten().cloned()
}

fn switch_of(m: &ArgMatches, id: &str) -> bool {
    m.try_get_one::<bool>(id)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

fn put(body: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        body.insert(key.to_string(), Value::String(value));
    }
}

/// `?key=value&...` for the pairs that have a value.
fn query(pairs: &[(&str, Option<String>)]) -> String {
    let mut query = String::new();
    for (key, value) in pairs {
        if let Some(value) = value {
            query.push(if query.is_empty() { '?' } else { '&' });
            let _ = write!(query, "{key}={}", encode(value));
        }
    }
    query
}

fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// `--base-url`, then `SPARK_API_BASE_URL`, then the saved target, then the
/// default; a source checkout refuses the default.
fn with_target(
    plan: ApiRequestPlan,
    names: &[&str],
    m: &ArgMatches,
    env: &impl Environment,
) -> Result<ApiRequestPlan, CommandOutput> {
    let flag = value_of(m, "base-url");
    let command = format!("spark {}", names.join(" "));
    match require_explicit_agent_base_url_with_env(
        &command,
        flag.as_deref(),
        &source_guard_root("spark"),
        env,
    ) {
        Ok(()) => {}
        Err(SparkCommonError::SourceCheckoutGuard(message)) => {
            return Err(json_error(message, EXIT_GENERAL_FAILURE))
        }
        Err(error) => return Err(json_error(error.to_string(), EXIT_GENERAL_FAILURE)),
    }
    let env_url = env
        .get_var("SPARK_API_BASE_URL")
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty());
    let (base_url, target_source) = match (flag, env_url) {
        (Some(url), _) => (url, "flag"),
        (None, Some(url)) => (url, "env"),
        (None, None) => {
            let fail = |error: String| json_error(error, EXIT_GENERAL_FAILURE);
            let bootstrap =
                spark_common::settings::resolve_settings_with_env(&Default::default(), env)
                    .map_err(|error| fail(error.to_string()))?;
            let (url, label) =
                spark_storage::settings::resolve_client_api_base_url(&bootstrap.config_dir, env)
                    .map_err(|error| fail(error.to_string()))?;
            (
                url,
                if label == "stored" {
                    "saved"
                } else {
                    "default"
                },
            )
        }
    };
    Ok(ApiRequestPlan {
        base_url,
        target_source,
        ..plan
    })
}

// ---------------------------------------------------------------------------
// Sending a request and printing its response.

/// The bearer a human sets to reach a remote server; agents on their own
/// server's loopback need none.
fn api_token(env: &impl Environment) -> Option<String> {
    env.get_var("SPARK_API_TOKEN")
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn execute(plan: &ApiRequestPlan, env: &impl Environment, out: &mut dyn Write) -> CommandOutput {
    let token = api_token(env);
    let token = token.as_deref();
    let payload = match &plan.view {
        View::Events {
            run_id,
            after,
            text,
            until_done,
        } => return stream_events(plan, run_id, *after, *text, *until_done, token, out),
        View::DelegationWait { timeout } => return wait_for_delegation(plan, *timeout, token),
        View::AnswerOnce | View::AnswerOpenQuestion => return answer_once(plan, token),
        _ => request_json(plan, token),
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(error) => return failure_output(plan, error, token),
    };
    match plan.view {
        View::FlowListText => CommandOutput::stdout(0, flow_list_text(&payload)),
        View::FlowFilesText => CommandOutput::stdout(0, flow_files_text(&payload)),
        View::FlowInputs => flow_inputs_output(&payload),
        View::TriggerListText => CommandOutput::stdout(0, trigger_list_text(&payload)),
        View::TriggerText => CommandOutput::stdout(0, trigger_text(&payload)),
        View::Validation { text } => flow_validation_output(&payload, text),
        _ => success_json(&payload),
    }
}

enum Failure {
    Transport(String),
    Http(u16, String),
}

fn transport(error: reqwest::Error) -> Failure {
    Failure::Transport(format!("Request failed: {error}"))
}

fn send(
    plan: &ApiRequestPlan,
    token: Option<&str>,
    timeout: Option<Duration>,
) -> Result<reqwest::blocking::Response, Failure> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(transport)?;
    let url = format!("{}{}", plan.base_url.trim_end_matches('/'), plan.path);
    let mut request = match plan.method {
        HttpMethod::Get => client.get(&url),
        HttpMethod::Post => client.post(&url),
        HttpMethod::Patch => client.patch(&url),
        HttpMethod::Delete => client.delete(&url),
    };
    if let Some(body) = &plan.body {
        request = request.json(body);
    }
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().map_err(transport)
}

/// The response, or the server's error from it.
fn check_status(
    response: reqwest::blocking::Response,
) -> Result<reqwest::blocking::Response, Failure> {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return Ok(response);
    }
    let text = response.text().map_err(transport)?;
    let payload = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    Err(Failure::Http(
        status.as_u16(),
        error_message_from_payload(&payload, &text),
    ))
}

fn read_payload(response: reqwest::blocking::Response) -> Result<Value, Failure> {
    let text = check_status(response)?.text().map_err(transport)?;
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "detail": text })))
}

fn request_json(plan: &ApiRequestPlan, token: Option<&str>) -> Result<Value, Failure> {
    read_payload(send(plan, token, Some(REQUEST_TIMEOUT))?)
}

/// The failure as JSON on stderr, naming the target, with the plan's hint
/// when the server's error is the one it answers.
fn failure_output(plan: &ApiRequestPlan, error: Failure, token: Option<&str>) -> CommandOutput {
    let target = Some((plan.base_url.as_str(), plan.target_source));
    let (status, mut message) = match error {
        Failure::Transport(message) => return failure(message, EXIT_GENERAL_FAILURE, None, target),
        Failure::Http(status, message) => (status, message),
    };
    match &plan.hint {
        Some(Hint::Flow {
            name,
            project,
            conversation,
        }) if status == 404 && message.contains("Unknown flow") => {
            let project = project.clone().or_else(|| {
                conversation
                    .as_deref()
                    .and_then(|handle| conversation_project(plan, handle, token))
            });
            if let Some(names) = similar_flows(plan, name, project, token) {
                let _ = write!(message, "; did you mean {names}?");
            }
        }
        Some(Hint::Retry { run_id })
            if status == 409 && message.contains("Retry requires a failed pipeline") =>
        {
            let _ = write!(
                message,
                "; use spark run continue {run_id} --from <node> to resume a run that did not fail"
            );
        }
        _ => {}
    }
    failure(message, http_status_exit_code(status), Some(status), target)
}

/// The project of the conversation with this handle.
// ponytail: reads every conversation's summary row, only after a flow 404;
// add a by-handle read route if this path gets hot.
fn conversation_project(
    plan: &ApiRequestPlan,
    handle: &str,
    token: Option<&str>,
) -> Option<String> {
    let list = ApiRequestPlan {
        method: HttpMethod::Get,
        path: "/workspace/api/projects/conversations".to_string(),
        body: None,
        ..plan.clone()
    };
    let conversations = request_json(&list, token).ok()?;
    conversations["rows"]
        .as_array()?
        .iter()
        .find(|row| row["kind"] == "conversation" && row["payload"]["handle"] == handle)
        .and_then(|row| row["payload"]["project_path"].as_str())
        .map(str::to_string)
}

/// Flows from `flow list` named `name`, or ending in `/<name>`, with any
/// `.dot` suffix dropped.
fn similar_flows(
    plan: &ApiRequestPlan,
    name: &str,
    project: Option<String>,
    token: Option<&str>,
) -> Option<String> {
    let wanted = name.strip_suffix(".dot").unwrap_or(name).trim_matches('/');
    let list = ApiRequestPlan {
        method: HttpMethod::Get,
        path: format!(
            "/workspace/api/flows{}",
            query(&[("project_path", project)])
        ),
        body: None,
        ..plan.clone()
    };
    let flows = request_json(&list, token).ok()?;
    let suffix = format!("/{wanted}");
    let names = flows
        .as_array()?
        .iter()
        .filter_map(|flow| flow["name"].as_str())
        .filter(|flow| *flow == wanted || flow.ends_with(&suffix))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join(" or "))
}

/// Prints each event as it arrives. Without `until_done`, stops at the
/// client's request timeout. With it, stops at the run's last journal entry
/// once the run has ended, so replayed ends of a retried run do not stop it;
/// a stream lost or lagging before then is an error.
fn stream_events(
    plan: &ApiRequestPlan,
    run_id: &str,
    after: Option<u64>,
    text: bool,
    until_done: bool,
    token: Option<&str>,
    out: &mut dyn Write,
) -> CommandOutput {
    let done = CommandOutput::stdout(0, String::new());
    let mut last = None;
    if until_done {
        last = match last_sequence_if_ended(plan, run_id, token) {
            Ok(last) => last,
            Err(error) => return failure_output(plan, error, token),
        };
        if last.is_some_and(|last| after.unwrap_or(0) >= last) {
            return done;
        }
    }
    let timeout = (!until_done).then_some(REQUEST_TIMEOUT);
    let response = match send(plan, token, timeout).and_then(check_status) {
        Ok(response) => response,
        Err(error) => return failure_output(plan, error, token),
    };
    let mut cursor = after;
    let mut data = Vec::new();
    let lines = BufReader::new(response).lines().map_while(Result::ok);
    for line in lines.chain([String::new()]) {
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
            continue;
        }
        if !line.is_empty() || data.is_empty() {
            continue;
        }
        let raw = data.join("\n");
        data.clear();
        let Ok(envelope @ Value::Object(_)) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let line = if text {
            run_event_text(&envelope)
        } else {
            format!("{envelope}\n")
        };
        if out
            .write_all(line.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            return done;
        }
        if envelope["cursor"]["kind"] == "run_sequence" {
            cursor = envelope["cursor"]["value"].as_u64().or(cursor);
        }
        if !until_done {
            continue;
        }
        // Dropped frames may include the run's end: hand back the cursor so
        // a rerun replays the rest from the journal.
        if envelope["type"] == "resync_required" {
            break;
        }
        if last.is_none() && is_run_end(&envelope) {
            last = match last_sequence_if_ended(plan, run_id, token) {
                Ok(last) => last,
                Err(error) => return failure_output(plan, error, token),
            };
        }
        if last.is_some_and(|last| cursor.unwrap_or(0) >= last) {
            return done;
        }
    }
    if !until_done {
        return done;
    }
    failure(
        format!(
            "The event stream for run {run_id} dropped events or ended before the run did; continue with spark run events {run_id} --after {} --until-done.",
            cursor.unwrap_or(0)
        ),
        EXIT_GENERAL_FAILURE,
        None,
        Some((&plan.base_url, plan.target_source)),
    )
}

/// An event that may end a root run: completion, failure, or the
/// `canceled` status, which is all a run canceled before it started writes.
// ponytail: a live run's cancel appends `PipelineFailed` right after its
// `canceled` status; a check that beats that append stops before it.
fn is_run_end(envelope: &Value) -> bool {
    let entry = &envelope["payload"];
    let ends = match entry["raw_type"].as_str() {
        Some("PipelineCompleted" | "PipelineFailed") => true,
        Some("runtime") => entry["payload"]["status"] == "canceled",
        _ => false,
    };
    envelope["type"] == "run.journal_entry" && ends && entry["source_scope"] != "child"
}

/// The sequence of the run's last journal entry, once the run has ended.
/// A retry reopens the run, so only its ended record says the journal is
/// complete.
fn last_sequence_if_ended(
    plan: &ApiRequestPlan,
    run_id: &str,
    token: Option<&str>,
) -> Result<Option<u64>, Failure> {
    let read = |path: String| ApiRequestPlan {
        method: HttpMethod::Get,
        path,
        body: None,
        ..plan.clone()
    };
    let run_path = format!("/attractor/pipelines/{}", encode(run_id));
    if request_json(&read(run_path.clone()), token)?["ended_at"].is_null() {
        return Ok(None);
    }
    let journal = request_json(&read(format!("{run_path}/journal?limit=1")), token)?;
    Ok(Some(journal["newest_sequence"].as_u64().unwrap_or(0)))
}

/// What one server poll of `spark delegation wait` asks for: under the
/// client's own request timeout.
const DELEGATION_SERVER_WAIT: Duration = Duration::from_secs(25);

/// Polls until the delegation seals or `timeout` passes (default: no limit).
/// Prints the record, whose `timed_out` and `result` a flow maps. Stopping
/// it changes nothing on either environment.
fn wait_for_delegation(
    plan: &ApiRequestPlan,
    timeout: Option<Duration>,
    token: Option<&str>,
) -> CommandOutput {
    let started = std::time::Instant::now();
    loop {
        let remaining = timeout.map(|timeout| timeout.saturating_sub(started.elapsed()));
        let wait = remaining.map_or(DELEGATION_SERVER_WAIT, |remaining| {
            remaining.min(DELEGATION_SERVER_WAIT)
        });
        let poll = ApiRequestPlan {
            path: format!("{}/wait?timeout_ms={}", plan.path, wait.as_millis()),
            ..plan.clone()
        };
        let record = match request_json(&poll, token) {
            Ok(record) => record,
            Err(error) => return failure_output(&poll, error, token),
        };
        let expired = timeout.is_some_and(|timeout| started.elapsed() >= timeout);
        if record["timed_out"] != true || expired {
            return success_json(&record);
        }
    }
}

/// Sends an answer once. A request whose response is lost is reported as
/// uncertain and never resent; `spark delegation get` shows whether the
/// question closed.
fn answer_once(plan: &ApiRequestPlan, token: Option<&str>) -> CommandOutput {
    let mut plan = plan.clone();
    if plan.view == View::AnswerOpenQuestion {
        let read = ApiRequestPlan {
            method: HttpMethod::Get,
            body: None,
            ..plan.clone()
        };
        let record = match request_json(&read, token) {
            Ok(record) => record,
            Err(error) => return failure_output(&read, error, token),
        };
        // The record keeps the target's latest view in `last_state`.
        let open = record["last_state"]["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|question| question["question_id"].as_str())
            .collect::<Vec<_>>();
        let [question] = open.as_slice() else {
            return json_error(
                format!(
                    "The delegation has {} open questions ({}); pass --question with one of them, from spark delegation get.",
                    open.len(),
                    open.join(", ")
                ),
                EXIT_GENERAL_FAILURE,
            );
        };
        plan.path = format!("{}/questions/{}/answer", plan.path, encode(question));
    }
    let response = match send(&plan, token, Some(REQUEST_TIMEOUT)) {
        Ok(response) => response,
        Err(Failure::Transport(error) | Failure::Http(_, error)) => {
            return failure(
                format!(
                    "The answer's outcome is unknown ({error}); it was not resent. Run spark delegation get to see whether the question closed."
                ),
                EXIT_GENERAL_FAILURE,
                None,
                Some((&plan.base_url, plan.target_source)),
            )
        }
    };
    match read_payload(response) {
        Ok(payload) => success_json(&payload),
        Err(error) => failure_output(&plan, error, token),
    }
}

/// The inputs `flow.md` declares, as a JSON array.
fn flow_inputs_output(payload: &Value) -> CommandOutput {
    let header = payload["files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|file| file["name"] == attractor_dsl::FLOW_FILE)
        .and_then(|file| file["content"].as_str());
    let Some(header) = header else {
        return json_error("The flow has no flow.md.", EXIT_GENERAL_FAILURE);
    };
    match attractor_dsl::parse_flow_file(header) {
        Ok(flow) => success_json(&json!(flow.inputs)),
        Err(error) => json_error(error.detail(), EXIT_GENERAL_FAILURE),
    }
}

/// Validates a flow directory as the loader assembles it, resolving roles
/// through `project` (else the project whose `.spark/definitions` holds the
/// directory), then the bundled library.
fn validate_flow_dir(
    dir: &str,
    project: Option<&str>,
    text: bool,
    env: &impl Environment,
) -> CommandOutput {
    let flow_path = expand_user_path(dir, env);
    if !flow_path.is_dir() {
        return json_error(
            format!("Flow directory not found: {dir}"),
            EXIT_GENERAL_FAILURE,
        );
    }
    let project = project
        .map(|project| expand_user_path(project, env))
        .or_else(|| {
            flow_path.ancestors().find_map(|dir| {
                let spark = dir.parent()?;
                (dir.file_name()? == "definitions" && spark.file_name()? == ".spark")
                    .then(|| spark.parent())
                    .flatten()
                    .map(Path::to_path_buf)
            })
        });
    let roots = match spark_common::settings::resolve_settings_with_env(&Default::default(), env) {
        Ok(settings) => attractor_dsl::FlowRoots::new(settings.bundled_dir(), project.as_deref()),
        Err(error) => return json_error(error.to_string(), EXIT_GENERAL_FAILURE),
    };
    let loaded = attractor_dsl::read_flow_dir(&flow_path).map_err(flow_source_validation_error);
    // Referenced roles must resolve; the preview keeps the authored form.
    let loaded = loaded.and_then(|flow| {
        attractor_dsl::resolve_flow_roles(&flow, &roots)
            .map(|_| flow)
            .map_err(flow_source_validation_error)
    });
    // Subflows validate against their effective children, as at launch.
    let flow_name = flow_path.ancestors().find_map(|dir| {
        (dir.file_name()? == "flows")
            .then(|| flow_path.strip_prefix(dir).ok())
            .flatten()
            .map(|name| name.to_string_lossy().replace('\\', "/"))
    });
    let loaded = loaded.and_then(|flow| {
        attractor_dsl::validate_flow_children(&roots, flow_name.as_deref(), &flow)
            .map(|_| flow)
            .map_err(|error| flow_source_validation_error(error.into()))
    });
    let mut response = match loaded {
        Ok(flow) => match flow_definition_preview_payload(&flow) {
            Value::Object(object) => object,
            _ => Map::new(),
        },
        Err((error, status)) => flow_definition_error_payload(&error, status),
    };
    response.insert(
        "name".to_string(),
        json!(flow_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()),
    );
    response.insert("path".to_string(), json!(resolve_display_path(&flow_path)));
    flow_validation_output(&Value::Object(response), text)
}

// ---------------------------------------------------------------------------
// Renderers and helpers.

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
        return "No flows found.\n".to_string();
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

/// A flow's layer and directory, then each file under a `==> <file> <==` header.
fn flow_files_text(payload: &Value) -> String {
    let field = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut text = format!("Layer: {}\nPath: {}\n", field("layer"), field("path"));
    for file in payload
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let content = file
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = file.get("name").and_then(Value::as_str).unwrap_or_default();
        text.push_str(&format!("\n==> {name} <==\n{content}"));
        if !content.ends_with('\n') {
            text.push('\n');
        }
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

/// The full report, exiting nonzero when it records a validation failure.
fn flow_validation_output(payload: &Value, text: bool) -> CommandOutput {
    let failed = payload["status"] == "validation_error"
        || payload["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|d| d["severity"] == "error"));
    let output = if text {
        CommandOutput::stdout(0, validate_flow_text(payload))
    } else {
        success_json(payload)
    };
    CommandOutput {
        exit_code: if failed { EXIT_GENERAL_FAILURE } else { 0 },
        ..output
    }
}

fn flow_source_validation_error(
    error: attractor_dsl::FlowSourceError,
) -> (FlowDefinitionError, &'static str) {
    let error = FlowDefinitionError {
        count: error.diagnostics().len().max(1),
        detail: error.detail().to_string(),
        diagnostics: error.diagnostics().to_vec(),
    };
    (error, "validation_error")
}

fn flow_definition_error_payload(error: &FlowDefinitionError, status: &str) -> Map<String, Value> {
    let mut diagnostics = error
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let mut payload = Map::from_iter([
                ("rule".to_string(), json!(diagnostic.rule_id)),
                ("rule_id".to_string(), json!(diagnostic.rule_id)),
                ("severity".to_string(), json!("error")),
                ("message".to_string(), json!(diagnostic.message)),
                ("line".to_string(), json!(0)),
                ("node".to_string(), Value::Null),
                ("node_id".to_string(), Value::Null),
            ]);
            if let Some(node_id) = &diagnostic.node_id {
                payload.insert("node".to_string(), json!(node_id));
                payload.insert("node_id".to_string(), json!(node_id));
            }
            if let Some((from, to)) = &diagnostic.edge {
                payload.insert("edge".to_string(), json!([from, to]));
            }
            Value::Object(payload)
        })
        .collect::<Vec<_>>();
    // A flow that fails to load has only its detail to report.
    if diagnostics.is_empty() {
        diagnostics.push(json!({
            "rule": "flow_definition",
            "rule_id": "flow_definition",
            "severity": "error",
            "message": error.detail,
            "line": 0,
            "node": null,
            "node_id": null,
        }));
    }
    Map::from_iter([
        ("status".to_string(), json!(status)),
        ("error".to_string(), json!(error.detail)),
        ("diagnostics".to_string(), json!(diagnostics)),
        ("errors".to_string(), json!(diagnostics)),
    ])
}

fn flow_definition_preview_payload(flow: &FlowDefinition) -> Value {
    let diagnostics = flow.diagnostics();
    let diagnostic_payloads = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut payload = Map::from_iter([
                ("rule".to_string(), json!(diagnostic.rule_id)),
                ("rule_id".to_string(), json!(diagnostic.rule_id)),
                ("severity".to_string(), json!("error")),
                ("message".to_string(), json!(diagnostic.message)),
                ("line".to_string(), json!(0)),
                ("node".to_string(), Value::Null),
                ("node_id".to_string(), Value::Null),
            ]);
            if let Some(node_id) = &diagnostic.node_id {
                payload.insert("node".to_string(), json!(node_id));
                payload.insert("node_id".to_string(), json!(node_id));
            }
            if let Some((from, to)) = &diagnostic.edge {
                payload.insert("edge".to_string(), json!([from, to]));
            }
            Value::Object(payload)
        })
        .collect::<Vec<_>>();
    let nodes = flow
        .nodes
        .iter()
        .map(|(node_id, node)| {
            json!({
                "id": node_id,
                "label": node.label,
                "kind": node.kind,
                "description": node.description,
                "config": node.config,
                "runtime": node.runtime,
                "contracts": node.contracts,
                "manager": node.manager,
                "retry": node.retry,
                "execution": node.execution,
                "ui": node.ui,
                "extensions": node.extensions,
            })
        })
        .collect::<Vec<_>>();
    let edges = flow
        .edges()
        .into_iter()
        .map(|edge| {
            json!({
                "source": edge.from,
                "target": edge.to,
                "from": edge.from,
                "to": edge.to,
                "kind": edge.kind,
                "decision": edge.decision,
                "description": edge.description,
                "label": edge.label,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "status": if diagnostics.is_empty() { "ok" } else { "validation_error" },
        "flow": flow.to_canonical_json_value(),
        "graph": {
            "id": flow.id,
            "title": flow.title,
            "description": flow.description,
            "goal": flow.goal,
            "nodes": nodes,
            "edges": edges,
            "metadata": flow.metadata,
        },
        "nodes": nodes,
        "edges": edges,
        "diagnostics": diagnostic_payloads,
        "errors": diagnostic_payloads,
        "child_previews": {},
    })
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

/// The action field naming a trigger's target: its mission, role, or flow.
fn action_target_key(action: &serde_json::Map<String, Value>) -> &'static str {
    match action.get("mode").and_then(Value::as_str) {
        Some("mission") => "mission_id",
        Some("role") => "role",
        _ => "flow_name",
    }
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
            .and_then(|action| action.get(action_target_key(action)))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let project_path = row
            .get("project_path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        writeln!(
            &mut text,
            "{trigger_id}: {name} [{source_type}] -> {flow_name}"
        )
        .expect("writing to String cannot fail");
        writeln!(
            &mut text,
            "  project={project_path} enabled={} protected={}",
            enabled, protected
        )
        .expect("writing to String cannot fail");
    }
    text
}

fn trigger_text(payload: &Value) -> String {
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
    let project_path = object
        .get("project_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
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
    writeln!(&mut text, "Project: {project_path}").expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "Source Type: {}",
        object
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
    writeln!(&mut text, "Enabled: {}", enabled).expect("writing to String cannot fail");
    writeln!(&mut text, "Protected: {}", protected).expect("writing to String cannot fail");
    writeln!(
        &mut text,
        "{} Target: {}",
        match action.and_then(|a| a.get("mode")).and_then(Value::as_str) {
            Some("mission") => "Mission",
            Some("role") => "Role",
            _ => "Flow",
        },
        action
            .and_then(|action| action.get(action_target_key(action)))
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .expect("writing to String cannot fail");
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

fn expand_user_path(value: &str, env: &impl Environment) -> PathBuf {
    if value.is_empty() {
        return PathBuf::from(".");
    }
    spark_common::paths::expand_tilde_with_env(Path::new(value), env)
}

fn resolve_display_path(path: &Path) -> String {
    spark_common::paths::canonicalize_clean(path)
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
