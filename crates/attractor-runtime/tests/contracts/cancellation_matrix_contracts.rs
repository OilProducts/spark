//! Cancellation and failure handling across parallel branches and subflows,
//! as one table: fork type x what the affected branch runs x stop trigger x
//! timing, each row checked against the design's precedence rules by one
//! shared verifier.
//!
//! The flow is always `start -> fork[pre -> (work | sub), sib] -> join ->
//! synth -> done`, and `sib -> failed` for a failure exit. `pre` is a finished
//! step, so a resume can be caught running it again. The affected branch is
//! index 0; `sib` (index 1) supplies the trigger. A subflow's child repeats the
//! shape as `cpre -> work -> cdone`.
//!
//! Synchronization is by signal files in the case's directory, and every wait
//! has a deadline: a hang fails the case instead of hanging the suite.
//!
//! Separately, one small table has the affected branch fail on its own while
//! the stop lands, which no row of the main table can do.
//!
//! Dropped combinations: none. Each of the 168 rows can occur:
//! - external x already finished needs work still in flight to cancel, which
//!   `sib` provides as a bystander;
//! - tolerant x tolerable error never stops the fork, so its "mid-execution"
//!   and "crash" rows assert that the affected branch runs on and the join
//!   runs once, rather than asserting a cancellation.
//!
//! Beyond the requested matrix, "subflow child" is split by what the child
//! runs, an agent or a tool, because the two stop through different paths, and
//! each tool also runs detached: its shell and a middle process exit while a
//! last descendant holds its output, so only the process group is left to stop.
//! A branch tool runs once more with a descendant that leaves the group and
//! closes the record it inherited, so only its output is left to find it by.
//!
//! Deliberately not asserted, because the design does not decide them: a
//! child whose own routing failure lands after its cancellation was requested
//! (the child's run finalizes as canceled), and a user canceling a child run
//! directly while its parent keeps running.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use attractor_core::{ContextMap, FailureKind, FlowDefinition, LaunchContext, RunRecord};
use attractor_runtime::{
    ExecuteRunRequest, ExecutionStart, PipelineExecutionResult, PipelineExecutor, RunStore,
    RuntimeControls, RuntimeHandlerRunner,
};
use serde_json::{json, Value};
use spark_agent_adapter::{CodergenBackendOutput, CodergenBackendRequest, CodergenError};

use super::{agent, exit, parse_flow, route};

const RUN: &str = "run-contract";
const DEADLINE: Duration = Duration::from_secs(60);
const CRASH_CASE_ENV: &str = "SPARK_CANCELLATION_MATRIX_CASE";
const CRASH_DIR_ENV: &str = "SPARK_CANCELLATION_MATRIX_DIR";
const CANCEL_DIR_ENV: &str = "SPARK_CANCELLATION_MATRIX_CANCEL_DIR";
const CANCEL_EXPECTS_ERROR_ENV: &str = "SPARK_CANCELLATION_MATRIX_CANCEL_EXPECTS_ERROR";
const AT_REGISTRATION_ENV: &str = "SPARK_CANCELLATION_MATRIX_AT_REGISTRATION";
const ISOLATED_DIR_ENV: &str = "SPARK_CANCELLATION_MATRIX_ISOLATED_DIR";
const CRASH_EXIT_CODE: i32 = 7;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fork {
    Strict,
    Tolerant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Running {
    Tool,
    /// A tool whose shell exits while a descendant runs on, holding its output.
    DetachedTool,
    /// A tool whose shell exits while a descendant in its own group runs on,
    /// holding only its output.
    LeftGroupTool,
    Agent,
    ChildAgent,
    ChildTool,
    ChildDetachedTool,
}

impl Running {
    fn is_child(self) -> bool {
        matches!(
            self,
            Self::ChildAgent | Self::ChildTool | Self::ChildDetachedTool
        )
    }

    /// The branch node the affected branch stops at.
    fn node(self) -> &'static str {
        if self.is_child() {
            "sub"
        } else {
            "work"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stop {
    External,
    Blocked,
    Fatal,
    Tolerable,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Timing {
    MidExecution,
    AlreadyFinished,
    CrashThenResume,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Case {
    fork: Fork,
    running: Running,
    stop: Stop,
    timing: Timing,
}

impl Case {
    fn name(self) -> String {
        format!(
            "{:?}/{:?}/{:?}/{:?}",
            self.fork, self.running, self.stop, self.timing
        )
    }

    /// Whether the trigger stops the fork from running the rest of its work.
    fn stops_fork(self) -> bool {
        !(self.fork == Fork::Tolerant && self.stop == Stop::Tolerable)
    }
}

fn cases(timing: Timing) -> Vec<Case> {
    let mut cases = Vec::new();
    for fork in [Fork::Strict, Fork::Tolerant] {
        for running in [
            Running::Tool,
            Running::DetachedTool,
            Running::LeftGroupTool,
            Running::Agent,
            Running::ChildAgent,
            Running::ChildTool,
            Running::ChildDetachedTool,
        ] {
            for stop in [Stop::External, Stop::Blocked, Stop::Fatal, Stop::Tolerable] {
                cases.push(Case {
                    fork,
                    running,
                    stop,
                    timing,
                });
            }
        }
    }
    cases
}

/// The owner's result under the design's precedence rules.
#[derive(Debug)]
struct Expected {
    status: &'static str,
    outcome: Option<&'static str>,
    result_name: Option<&'static str>,
    kind: Option<FailureKind>,
    reason: &'static str,
    current_node: Option<&'static str>,
}

fn expected(case: Case) -> Expected {
    let (status, outcome, result_name, kind, reason, current_node) = match (case.stop, case.fork) {
        // External cancellation keeps the aborted result on any fork.
        (Stop::External, _) => ("canceled", None, None, None, "aborted_by_user", None),
        // A failure exit is a completed domain result.
        (Stop::Blocked, _) => (
            "completed",
            Some("failure"),
            Some("blocked"),
            None,
            "",
            Some("failed"),
        ),
        // A routing error is fatal on either fork.
        (Stop::Fatal, _) => (
            "failed",
            None,
            None,
            Some(FailureKind::Routing),
            "branch sib (index 1) failed at sib: visit_limit_reached",
            None,
        ),
        (Stop::Tolerable, Fork::Strict) => (
            "failed",
            None,
            None,
            Some(FailureKind::Runtime),
            "branch sib (index 1) failed at sib: ",
            None,
        ),
        (Stop::Tolerable, Fork::Tolerant) => (
            "completed",
            Some("success"),
            Some("completed"),
            None,
            "",
            Some("done"),
        ),
    };
    Expected {
        status,
        outcome,
        result_name,
        kind,
        reason,
        current_node,
    }
}

/// One case's directory: signal files, logs and the run store.
#[derive(Clone)]
struct Signals {
    dir: PathBuf,
}

impl Signals {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn raise(&self, name: &str) {
        std::fs::write(self.path(name), "").unwrap();
    }

    fn raised(&self, name: &str) -> bool {
        self.path(name).exists()
    }

    fn log_call(&self, node: &str) {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path("calls.log"))
            .unwrap();
        // One write: tools append here too, and `writeln!` writes the line and
        // its newline separately, so a tool's line could land between them.
        log.write_all(format!("{node}\n").as_bytes()).unwrap();
    }

    fn calls(&self, node: &str) -> usize {
        std::fs::read_to_string(self.path("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == node)
            .count()
    }

    fn store(&self) -> RunStore {
        RunStore::for_runs_dir(self.path("runs"))
    }

    fn branches_dir(&self) -> Option<PathBuf> {
        let paths = self.store().find_run_root(RUN).ok().flatten()?;
        Some(paths.logs_dir().join("fork/executions/1-0/branches"))
    }

    fn branch_settled(&self, index: usize) -> bool {
        self.branches_dir()
            .is_some_and(|dir| dir.join(format!("{index}.json")).exists())
    }

    fn branch_record(&self, index: usize) -> Value {
        let path = self
            .branches_dir()
            .expect("run root")
            .join(format!("{index}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("branch record {}: {error}", path.display()));
        serde_json::from_str(&text).unwrap()
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Runs `work` on its own thread; a hang fails the case at the deadline.
fn bounded<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(DEADLINE)
        .unwrap_or_else(|_| panic!("{what} did not settle before the deadline"))
}

/// The fake agent. `work` holds until released or stopped, and `sib` plays
/// the case's trigger. While `hold` is raised (the first life of a crash case)
/// agents ignore cancellation, so the crash lands before settlement.
struct Backend {
    case: Case,
    signals: Signals,
    cancel: Option<spark_agent_adapter::codergen::CancelCheck>,
}

impl Backend {
    fn answer(decision: &str, notes: &str) -> Result<CodergenBackendOutput, CodergenError> {
        Ok(CodergenBackendOutput::text(
            json!({"decision": decision, "notes": notes}).to_string(),
        ))
    }

    fn canceled(&self) -> bool {
        !self.signals.raised("hold") && self.cancel.as_ref().is_some_and(|canceled| canceled())
    }

    /// Waits for `signal`, or answers with the cancellation a real backend
    /// reports when its node is stopped.
    fn hold_until(&self, signal: &str) -> Result<(), CodergenError> {
        if !wait_for(|| self.signals.raised(signal) || self.canceled()) {
            return Err(CodergenError::Backend(format!(
                "deadline: never saw {signal} or a stop"
            )));
        }
        if self.signals.raised(signal) {
            return Ok(());
        }
        Err(CodergenError::Backend(
            "Run canceled while the node was running".to_string(),
        ))
    }
}

impl spark_agent_adapter::CodergenBackend for Backend {
    fn set_cancel_check(&mut self, check: spark_agent_adapter::codergen::CancelCheck) {
        self.cancel = Some(check);
    }

    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        self.signals.log_call(&request.node_id);
        match request.node_id.as_str() {
            "work" => {
                self.signals.raise("started");
                if self.signals.raised("own-error") {
                    wait_for(|| self.signals.raised("stop-landed"));
                    return Err(CodergenError::Backend("work's own failure".to_string()));
                }
                self.hold_until("release")?;
                self.signals.log_call("work-finished");
                Self::answer("done", "work finished")
            }
            "sib" => match self.case.stop {
                // A bystander: running until the cancellation reaches it.
                Stop::External => {
                    self.hold_until("never")?;
                    Self::answer("done", "sib finished")
                }
                Stop::Blocked => {
                    self.hold_until("trigger")?;
                    Self::answer("blocked", "sib cannot proceed")
                }
                Stop::Fatal => {
                    self.hold_until("trigger")?;
                    Self::answer("again", "sib selected an exhausted target")
                }
                Stop::Tolerable => {
                    self.hold_until("trigger")?;
                    Err(CodergenError::Backend("provider down".to_string()))
                }
            },
            "synth" => Self::answer("done", "synthesized"),
            other => panic!("no fake answer for {other}"),
        }
    }
}

fn runner(case: Case, signals: &Signals) -> RuntimeHandlerRunner {
    let signals = signals.clone();
    RuntimeHandlerRunner::new().with_codergen_backend_factory(move || {
        Box::new(Backend {
            case,
            signals: signals.clone(),
            cancel: None,
        })
    })
}

fn tool(command: String, next: &str) -> Value {
    json!({"kind": "tool", "config": {"kind": "tool", "command": command, "exit_codes": {"0": "done"}},
        "routes": {"done": route(next)}})
}

fn step_tool(signals: &Signals, node: &str, next: &str) -> Value {
    let log = signals.path("calls.log");
    tool(
        format!("echo {node} >> '{}'; echo {node}-output", log.display()),
        next,
    )
}

/// Records the shell's process group for the stopped-process assertions.
const RECORD_GROUP: &str = "ps -o pgid= -p $$ | tr -d ' '";

/// While `hold` is raised, the commands that make a shell survive TERM, so only
/// a KILL settles it once the executor is gone. TERM leaves a signal. Stderr
/// goes first: with the executor gone, the shell's notice of a killed job would
/// die on the closed pipe.
fn survive_term(signals: &Signals) -> String {
    let p = |name: &str| signals.path(name).display().to_string();
    format!(
        "if [ -f '{hold}' ]; then exec 2>/dev/null; trap \"touch '{seen}'\" TERM; fi",
        hold = p("hold"),
        seen = p("term-seen"),
    )
}

/// Holds until released, bounded so a stop that never arrives shows up as a
/// finished tool.
fn hold_work(signals: &Signals) -> String {
    let p = |name: &str| signals.path(name).display().to_string();
    format!(
        "touch '{started}'; i=0; while [ ! -f '{release}' ] && [ $i -lt 1800 ]; \
         do sleep 0.05; i=$((i+1)); done; echo work-finished >> '{calls}'",
        started = p("started"),
        release = p("release"),
        calls = p("calls.log"),
    )
}

/// A `work` tool: announces its group and holds, signaling as early as a tool
/// can, with no wait for anything the runtime records.
fn work_tool(signals: &Signals, next: &str) -> Value {
    let p = |name: &str| signals.path(name).display().to_string();
    tool(
        format!(
            "{survive}; echo work >> '{calls}'; {RECORD_GROUP} >> '{pids}'; {hold}",
            survive = survive_term(signals),
            calls = p("calls.log"),
            pids = p("pids"),
            hold = hold_work(signals),
        ),
        next,
    )
}

/// A `work` tool whose processes turn over: the shell starts a middle process
/// and exits, the middle starts the holder and exits, and the holder, after
/// seeing both gone, closes the inherited marker and holds their output. Only
/// its process group is left to find it by.
fn detached_work_tool(signals: &Signals, next: &str) -> Value {
    let p = |name: &str| signals.path(name).display().to_string();
    let middle = signals.path("middle.sh");
    std::fs::write(
        &middle,
        format!(
            "( {survive}; exec 9<&-; \
             gone() {{ ! ps -o stat= -p \"$1\" | grep -qv Z; }}; \
             while ! gone \"$1\" || ! gone $$; do sleep 0.02; done; {hold} ) & exit 0\n",
            survive = survive_term(signals),
            hold = hold_work(signals),
        ),
    )
    .unwrap();
    tool(
        format!(
            "echo work >> '{calls}'; {RECORD_GROUP} >> '{pids}'; sh '{middle}' $$ & exit 0",
            calls = p("calls.log"),
            pids = p("pids"),
            middle = middle.display(),
        ),
        next,
    )
}

/// A `work` tool whose descendant leaves the group for its own, closes the
/// record it inherited, and outlives the shell. `set -m` needs a shell with job
/// control; elsewhere the row runs as the in-group one.
fn left_group_work_tool(signals: &Signals, next: &str) -> Value {
    let p = |name: &str| signals.path(name).display().to_string();
    tool(
        format!(
            "set -m; echo work >> '{calls}'; {RECORD_GROUP} >> '{pids}'; \
             ( {survive}; exec 9<&-; while ps -o stat= -p $$ | grep -qv Z; do sleep 0.02; done; \
             {hold} ) & echo $! >> '{pids}'; exit 0",
            calls = p("calls.log"),
            pids = p("pids"),
            survive = survive_term(signals),
            hold = hold_work(signals),
        ),
        next,
    )
}

fn flow(case: Case, signals: &Signals) -> FlowDefinition {
    let tolerate = case.fork == Fork::Tolerant;
    let mut nodes = json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["pre", "sib"],
            "join": "join", "tolerate_branch_execution_errors": tolerate}},
        "pre": step_tool(signals, "pre", case.running.node()),
        "sib": {"kind": "agent_task", "contracts": {"response": "decision_envelope"},
            "runtime": {"max_visits": 1},
            "routes": {"done": route("join"), "blocked": route("failed"), "again": route("sib")}},
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    });
    nodes[case.running.node()] = match case.running {
        Running::Tool => work_tool(signals, "join"),
        Running::DetachedTool => detached_work_tool(signals, "join"),
        Running::LeftGroupTool => left_group_work_tool(signals, "join"),
        Running::Agent => agent(json!({"done": route("join")})),
        Running::ChildAgent | Running::ChildTool | Running::ChildDetachedTool => json!({
            "kind": "subflow", "manager": {"poll_interval": "0s"},
            "config": {"kind": "subflow", "flow_ref": "child", "results": {"completed": "child_completed"}},
            "routes": {"child_completed": route("join")}}),
    };
    parse_flow(nodes)
}

fn write_child_flow(case: Case, signals: &Signals) {
    let work = match case.running {
        Running::ChildTool => work_tool(signals, "cdone"),
        Running::ChildDetachedTool => detached_work_tool(signals, "cdone"),
        _ => agent(json!({"done": route("cdone")})),
    };
    let flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("cpre")}},
        "cpre": step_tool(signals, "cpre", "work"),
        "work": work,
        "cdone": exit("completed", "success"),
    }));
    attractor_dsl::write_flow_dir(&signals.dir.join("flows/child"), &flow).unwrap();
}

/// The executor as production runs it, watching its own record for control
/// requests.
fn executor(
    runner: RuntimeHandlerRunner,
    store: &RunStore,
) -> PipelineExecutor<RuntimeHandlerRunner> {
    let control_store = store.clone();
    PipelineExecutor::with_control(runner, move || {
        let paths = control_store.find_run_root(RUN).ok().flatten()?;
        attractor_runtime::disk_execution_control(paths)()
    })
}

fn fresh_request(flow: FlowDefinition, signals: &Signals) -> ExecuteRunRequest {
    ExecuteRunRequest {
        store: signals.store(),
        record: RunRecord::new(RUN, signals.dir.to_string_lossy()),
        flow,
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::from([(
            "internal.bundled_root".to_string(),
            json!(signals.dir.to_string_lossy()),
        )]),
        max_steps: None,
        start: Default::default(),
    }
}

fn request_external_cancel(signals: &Signals) {
    RuntimeControls::new(signals.store())
        .request_cancel(RUN)
        .expect("cancel request");
}

/// Drives one case's first life from outside the executor: waits for the
/// affected branch to reach the case's timing, fires the trigger, then either
/// crashes the process or lets a surviving branch finish.
fn conduct(case: Case, signals: Signals) {
    let reached = wait_for(|| match case.timing {
        Timing::AlreadyFinished => signals.branch_settled(0),
        Timing::MidExecution | Timing::CrashThenResume => signals.raised("started"),
    });
    if !reached {
        return;
    }
    signals.raise("trigger");
    if case.stop == Stop::External {
        request_external_cancel(&signals);
    } else if !wait_for(|| signals.branch_settled(1)) {
        return;
    }
    if case.timing == Timing::CrashThenResume {
        std::process::exit(CRASH_EXIT_CODE);
    }
    if !case.stops_fork() {
        signals.raise("release");
    }
}

fn run_live(case: Case, signals: &Signals) -> PipelineExecutionResult {
    if case.timing == Timing::AlreadyFinished {
        signals.raise("release");
    }
    let conductor_signals = signals.clone();
    std::thread::spawn(move || conduct(case, conductor_signals));
    let (runner, store) = (runner(case, signals), signals.store());
    let request = fresh_request(flow(case, signals), signals);
    bounded(&case.name(), move || {
        executor(runner, &store).execute(request).expect("execute")
    })
}

/// Child process entry point of a crash case: runs the first life until the
/// conductor kills the process. A no-op in the ordinary test run.
#[test]
fn crash_case_first_life() {
    let Ok(name) = std::env::var(CRASH_CASE_ENV) else {
        return;
    };
    let signals = Signals {
        dir: PathBuf::from(std::env::var(CRASH_DIR_ENV).unwrap()),
    };
    let case = cases(Timing::CrashThenResume)
        .into_iter()
        .find(|case| case.name() == name)
        .expect("known case");
    let conductor_signals = signals.clone();
    if std::env::var(AT_REGISTRATION_ENV).is_ok() {
        std::thread::spawn(move || {
            if wait_for(|| conductor_signals.raised("at-registration")) {
                std::process::exit(CRASH_EXIT_CODE);
            }
        });
    } else {
        std::thread::spawn(move || conduct(case, conductor_signals));
    }
    let _ = executor(runner(case, &signals), &signals.store())
        .execute(fresh_request(flow(case, &signals), &signals));
    panic!("the first life should have crashed");
}

fn run_first_life_until_crash(case: Case, signals: &Signals) {
    run_first_life(
        case,
        signals,
        Command::new(std::env::current_exe().unwrap()),
    );
}

fn run_first_life(case: Case, signals: &Signals, mut command: Command) {
    signals.raise("hold");
    let mut child = command
        .args([
            "--exact",
            "routing_model_contracts::cancellation_matrix_contracts::crash_case_first_life",
            "--nocapture",
        ])
        .env(CRASH_CASE_ENV, case.name())
        .env(CRASH_DIR_ENV, &signals.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut status = None;
    wait_for(|| {
        status = child.try_wait().unwrap();
        status.is_some()
    });
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("{} first life did not crash", case.name());
    }
    assert_eq!(
        status.unwrap().code(),
        Some(CRASH_EXIT_CODE),
        "{} first life must die at the crash point",
        case.name()
    );
    std::fs::remove_file(signals.path("hold")).unwrap();
}

fn resume(
    runner: RuntimeHandlerRunner,
    flow: FlowDefinition,
    store: RunStore,
) -> PipelineExecutionResult {
    let bundle = store.read_run_bundle(RUN).unwrap().unwrap();
    let request = ExecuteRunRequest {
        store: store.clone(),
        record: bundle.record.unwrap(),
        flow,
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::new(),
        max_steps: None,
        start: ExecutionStart::Resume {
            paths: bundle.paths,
            checkpoint: bundle.checkpoint.unwrap(),
        },
    };
    executor(runner, &store).execute(request).expect("resume")
}

fn run_crash_then_resume(case: Case, signals: &Signals) -> PipelineExecutionResult {
    run_first_life_until_crash(case, signals);
    // The orphan from the first life must be stopped, not released: only an
    // invocation the resume relaunches may finish.
    if !case.stops_fork() {
        let signals = signals.clone();
        std::thread::spawn(move || {
            if wait_for(|| signals.calls("work") == 2) {
                signals.raise("release");
            }
        });
    }
    let (runner, store) = (runner(case, signals), signals.store());
    let flow = flow(case, signals);
    bounded(&case.name(), move || resume(runner, flow, store))
}

/// Whether a process in this group has not yet exited.
fn group_running(group: &str) -> bool {
    let output = Command::new("ps")
        .args(["-A", "-o", "pgid=,stat="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some(group) && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
    })
}

fn tool_groups(signals: &Signals) -> Vec<String> {
    std::fs::read_to_string(signals.path("pids"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Every process a tool started stopped. A stopped tool is gone when the run
/// returns; a tool the case released finishes as its output closes.
fn assert_processes_stopped(signals: &Signals, name: &str, stopped: bool) {
    for group in tool_groups(signals) {
        assert!(
            if stopped {
                !group_running(&group)
            } else {
                wait_for(|| !group_running(&group))
            },
            "{name}: tool process group {group} still runs"
        );
    }
}

/// Every ownership entry under the case's run roots, children included.
fn tool_ownership_entries(signals: &Signals) -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, entries: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, entries);
            } else if dir.ends_with("tool-processes") {
                entries.push(path);
            }
        }
    }
    let mut entries = Vec::new();
    walk(&signals.path("runs"), &mut entries);
    entries
}

fn verify(case: Case, signals: &Signals, result: &PipelineExecutionResult) {
    let name = case.name();
    let want = expected(case);
    let store = signals.store();
    let paths = store.find_run_root(RUN).unwrap().unwrap();
    let record = store.read_run_record(&paths).unwrap().unwrap();

    // The owner's final result, returned and durable.
    assert_eq!(
        result.status, want.status,
        "{name}: {}",
        result.failure_reason
    );
    assert_eq!(record.status, want.status, "{name}");
    assert_eq!(result.outcome.as_deref(), want.outcome, "{name}");
    assert_eq!(
        result.outcome_reason_code.as_deref(),
        want.result_name,
        "{name}"
    );
    assert_eq!(result.failure_kind, want.kind, "{name}");
    assert_eq!(record.failure_kind, want.kind, "{name}");
    assert!(
        result.failure_reason.starts_with(want.reason),
        "{name}: {}",
        result.failure_reason
    );
    if case.stop == Stop::Tolerable && case.fork == Fork::Strict {
        assert!(
            result.failure_reason.contains("provider down"),
            "{name}: {}",
            result.failure_reason
        );
    }
    if let Some(node) = want.current_node {
        assert_eq!(result.current_node, node, "{name}");
    }
    if case.stop == Stop::Blocked {
        let message = result.outcome_reason_message.clone().unwrap_or_default();
        assert!(
            message.contains("branch sib (index 1)") && message.contains("sib cannot proceed"),
            "{name}: {message}"
        );
    }

    // Every process and child stopped and settled; one child, never relaunched.
    let affected_stopped = case.stops_fork() && case.timing != Timing::AlreadyFinished;
    assert_processes_stopped(signals, &name, affected_stopped);
    assert_eq!(
        tool_ownership_entries(signals),
        Vec::<PathBuf>::new(),
        "{name}"
    );
    let children = store
        .list_run_records()
        .unwrap()
        .into_iter()
        .filter(|child| child.parent_run_id.as_deref() == Some(RUN))
        .collect::<Vec<_>>();
    if case.running.is_child() {
        assert_eq!(children.len(), 1, "{name}: {children:?}");
        assert_eq!(
            children[0].status,
            if affected_stopped {
                "canceled"
            } else {
                "completed"
            },
            "{name}"
        );
    } else {
        assert!(children.is_empty(), "{name}: {children:?}");
    }

    // The affected branch's durable record: settled as the case demands, with
    // its context, trace and visit counts intact.
    let affected = signals.branch_record(0);
    let node = case.running.node();
    assert_eq!(affected["id"], json!("pre"), "{name}");
    assert!(
        affected["context"]["context.tool.exit_code"].is_number(),
        "{name}: pre's context is gone: {affected}"
    );
    assert_eq!(affected["visits"]["pre"], json!(1), "{name}: {affected}");
    assert_eq!(affected["visits"][node], json!(1), "{name}: {affected}");
    if affected_stopped {
        assert_eq!(affected["status"], json!("canceled"), "{name}: {affected}");
        assert!(affected.get("error").is_none(), "{name}: {affected}");
        assert_eq!(affected["canceled_at"], json!(node), "{name}: {affected}");
        assert_eq!(affected["route_trace"], json!(["pre", node]), "{name}");
        if case.timing == Timing::MidExecution {
            let interruption = affected["interruption"].as_str().unwrap_or_default();
            assert!(!interruption.is_empty(), "{name}: {affected}");
        }
    } else {
        assert_eq!(affected["status"], json!("completed"), "{name}: {affected}");
        assert_eq!(
            affected["route_trace"],
            json!(["pre", node, "join"]),
            "{name}"
        );
    }

    // The trigger's own record keeps its class and evidence.
    let sib = signals.branch_record(1);
    assert_eq!(sib["id"], json!("sib"), "{name}");
    match case.stop {
        Stop::External => {
            assert_eq!(sib["status"], json!("canceled"), "{name}: {sib}");
            assert!(sib.get("error").is_none(), "{name}: {sib}");
        }
        Stop::Blocked => {
            assert_eq!(sib["status"], json!("terminal_result"), "{name}: {sib}");
            assert_eq!(sib["terminal"]["exit_id"], json!("failed"), "{name}");
            assert_eq!(
                sib["terminal"]["result"]["name"],
                json!("blocked"),
                "{name}"
            );
            assert_eq!(sib["terminal"]["node"], json!("sib"), "{name}");
            assert_eq!(sib["terminal"]["decision"], json!("blocked"), "{name}");
            assert_eq!(sib["route_trace"], json!(["sib", "failed"]), "{name}");
        }
        Stop::Fatal => {
            assert_eq!(sib["status"], json!("error"), "{name}: {sib}");
            assert_eq!(sib["error"]["kind"], json!("routing"), "{name}");
            assert_eq!(sib["error"]["tolerable"], json!(false), "{name}");
            assert_eq!(sib["error"]["node"], json!("sib"), "{name}");
        }
        Stop::Tolerable => {
            assert_eq!(sib["status"], json!("error"), "{name}: {sib}");
            assert_eq!(sib["error"]["kind"], json!("runtime"), "{name}");
            assert_eq!(sib["error"]["tolerable"], json!(true), "{name}");
            assert_eq!(sib["error"]["attempts"], json!(2), "{name}: {sib}");
            assert!(
                sib["error"]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("provider down"),
                "{name}: {sib}"
            );
        }
    }
    assert_eq!(sib["visits"]["sib"], json!(1), "{name}: {sib}");
    assert!(
        sib["context"]["current_node"] == json!("sib"),
        "{name}: sib's context is gone: {sib}"
    );
    if case.stop != Stop::Blocked {
        assert_eq!(sib["route_trace"], json!(["sib"]), "{name}: {sib}");
    }

    // Completed steps ran once, and a stopped invocation never ran on.
    assert_eq!(signals.calls("pre"), 1, "{name}: pre ran again");
    if case.running.is_child() {
        assert_eq!(signals.calls("cpre"), 1, "{name}: cpre ran again");
    }
    // Only a crash with no stop may relaunch the interrupted invocation.
    let relaunched = case.timing == Timing::CrashThenResume && !case.stops_fork();
    assert_eq!(
        signals.calls("work"),
        if relaunched { 2 } else { 1 },
        "{name}: work invocations"
    );
    assert_eq!(
        signals.calls("work-finished"),
        usize::from(!affected_stopped),
        "{name}: a stopped invocation ran to its end"
    );
    assert_eq!(
        signals.calls("sib"),
        if case.stop == Stop::Tolerable { 2 } else { 1 },
        "{name}: sib invocations"
    );

    // The join runs at most once, and never after a stop.
    let joined = usize::from(!case.stops_fork());
    let events = store.read_raw_events(&paths).unwrap();
    let stages = |node: &str| {
        events
            .iter()
            .filter(|event| {
                event.event_type == "StageStarted"
                    && event.payload.get("node_id") == Some(&json!(node))
            })
            .count()
    };
    assert_eq!(stages("join"), joined, "{name}: join stages");
    assert_eq!(signals.calls("synth"), joined, "{name}: synth invocations");
    let checkpoint = store.read_checkpoint(&paths).unwrap().unwrap();
    assert_eq!(checkpoint.visit_counts["fork"], 1, "{name}: fork visits");
    assert_eq!(
        checkpoint.visit_counts.get("join").copied().unwrap_or(0),
        joined as u64,
        "{name}: join visits"
    );

    // No branch's context reached the parent.
    assert!(
        !result.context.contains_key("context.tool.exit_code"),
        "{name}: a branch's context was promoted"
    );
    // An external stop finalizes before the fork's outcome applies, so its
    // evidence is the durable records checked above.
    if case.stop != Stop::External {
        let results = result.context["parallel.results"].as_array().unwrap();
        assert_eq!(results.len(), 2, "{name}");
        assert_eq!(results[0], affected, "{name}");
        assert_eq!(results[1], sib, "{name}");
    }
}

fn run_case(case: Case) {
    let temp = tempfile::tempdir().unwrap();
    let signals = Signals {
        dir: temp.path().to_path_buf(),
    };
    if case.running.is_child() {
        write_child_flow(case, &signals);
    }
    let result = match case.timing {
        Timing::CrashThenResume => run_crash_then_resume(case, &signals),
        _ => run_live(case, &signals),
    };
    verify(case, &signals, &result);
}

/// Runs the table on a small pool, collecting every failing row. One table at
/// a time: this binary's other tests assert on wall-clock promptness.
fn run_table(cases: Vec<Case>) {
    static ONE_TABLE: Mutex<()> = Mutex::new(());
    let _one_table = ONE_TABLE.lock().unwrap_or_else(|error| error.into_inner());
    let next = Arc::new(AtomicUsize::new(0));
    let cases = Arc::new(cases);
    let failures = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let (next, cases, failures) = (next.clone(), cases.clone(), failures.clone());
            scope.spawn(move || {
                while let Some(case) = cases.get(next.fetch_add(1, Ordering::SeqCst)).copied() {
                    if let Err(payload) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(case)))
                    {
                        let message = payload
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "panic".to_string());
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {message}", case.name()));
                    }
                }
            });
        }
    });
    let failures = failures.lock().unwrap();
    assert!(
        failures.is_empty(),
        "{} rows failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn matrix_branch_stopped_mid_execution() {
    run_table(cases(Timing::MidExecution));
}

#[test]
fn matrix_branch_already_finished() {
    run_table(cases(Timing::AlreadyFinished));
}

#[test]
fn matrix_crash_before_settlement_then_resume() {
    run_table(cases(Timing::CrashThenResume));
}

/// A branch failing on its own as a stop lands keeps its error, is not retried
/// after the stop, and the owner's result follows the precedence rules.
#[test]
fn an_independent_error_during_a_stop_stays_an_error() {
    // Tolerant x tolerable never stops the fork, and two fatal errors on a
    // strict fork have no primary the design decides, so `reason` is open there.
    // Agents only: a stopped tool is killed, so it cannot then fail on its own,
    // and a child's own failure is covered by routing_model_contracts'
    // a_child_routing_failure_stays_fatal_inside_a_tolerant_fork,
    // a_child_execution_error_stays_tolerated_inside_a_tolerant_fork and
    // an_independent_child_failure_after_sibling_cancellation_stays_an_error.
    let rows = [
        (Fork::Strict, Stop::External, "canceled", "aborted_by_user"),
        (
            Fork::Tolerant,
            Stop::External,
            "canceled",
            "aborted_by_user",
        ),
        (
            Fork::Strict,
            Stop::Blocked,
            "failed",
            "branch pre (index 0)",
        ),
        (Fork::Tolerant, Stop::Blocked, "completed", ""),
        (Fork::Strict, Stop::Fatal, "failed", ""),
        (
            Fork::Tolerant,
            Stop::Fatal,
            "failed",
            "branch sib (index 1)",
        ),
        (Fork::Strict, Stop::Tolerable, "failed", ""),
    ];
    for (fork, stop, status, reason) in rows {
        let case = Case {
            fork,
            running: Running::Agent,
            stop,
            timing: Timing::MidExecution,
        };
        let name = case.name();
        let temp = tempfile::tempdir().unwrap();
        let signals = Signals {
            dir: temp.path().to_path_buf(),
        };
        signals.raise("own-error");
        let conductor = signals.clone();
        std::thread::spawn(move || {
            if wait_for(|| conductor.raised("started")) {
                conductor.raise("trigger");
                if stop == Stop::External {
                    request_external_cancel(&conductor);
                }
                wait_for(|| conductor.branch_settled(1));
                conductor.raise("stop-landed");
            }
        });
        let (runner, store) = (runner(case, &signals), signals.store());
        let request = fresh_request(flow(case, &signals), &signals);
        let result = bounded(&name, move || {
            executor(runner, &store).execute(request).expect("execute")
        });

        assert_eq!(result.status, status, "{name}: {}", result.failure_reason);
        assert!(
            result.failure_reason.starts_with(reason),
            "{name}: {}",
            result.failure_reason
        );
        if stop == Stop::Blocked && fork == Fork::Tolerant {
            assert_eq!(
                result.outcome_reason_code.as_deref(),
                Some("blocked"),
                "{name}"
            );
        }
        let affected = signals.branch_record(0);
        assert_eq!(affected["status"], json!("error"), "{name}: {affected}");
        assert_eq!(affected["error"]["kind"], json!("runtime"), "{name}");
        assert_eq!(
            affected["error"]["attempts"],
            json!(1),
            "{name}: {affected}"
        );
        assert!(
            affected["error"]["reason"]
                .as_str()
                .unwrap()
                .contains("work's own failure"),
            "{name}: {affected}"
        );
        assert!(affected.get("canceled_at").is_none(), "{name}: {affected}");
        assert_eq!(signals.calls("work"), 1, "{name}: retried after the stop");
        assert_eq!(signals.calls("synth"), 0, "{name}: joined after the stop");
    }
}

fn in_new_case_dir(case: Case) -> (tempfile::TempDir, Signals) {
    let temp = tempfile::tempdir().unwrap();
    let signals = Signals {
        dir: temp.path().to_path_buf(),
    };
    if case.running.is_child() {
        write_child_flow(case, &signals);
    }
    (temp, signals)
}

/// A crash with no stop in flight, so the tool has seen no TERM yet.
fn crashed_running_case(running: Running) -> Case {
    Case {
        fork: Fork::Tolerant,
        running,
        stop: Stop::Tolerable,
        timing: Timing::CrashThenResume,
    }
}

/// Child process entry point: runs a real orphan cancellation and dies as soon
/// as the tool has been told to stop. A no-op in the ordinary test run.
#[test]
fn cancellation_of_a_crashed_run_dies_midway() {
    let Ok(dir) = std::env::var(CANCEL_DIR_ENV) else {
        return;
    };
    let signals = Signals {
        dir: PathBuf::from(dir),
    };
    if std::env::var(CANCEL_EXPECTS_ERROR_ENV).is_ok() {
        RuntimeControls::new(signals.store())
            .mark_canceled(RUN, "no executor")
            .expect_err("settlement with no process tools must fail");
        std::process::exit(CRASH_EXIT_CODE);
    }
    let watcher = signals.clone();
    std::thread::spawn(move || {
        if wait_for(|| watcher.raised("term-seen")) {
            std::process::exit(CRASH_EXIT_CODE);
        }
    });
    let outcome = RuntimeControls::new(signals.store()).mark_canceled(RUN, "no executor");
    panic!("the cancellation should have died while stopping the tool: {outcome:?}");
}

/// A dead executor's run: canceling it publishes no terminal state until its
/// tool, branches and child have settled; a cancellation dying midway publishes
/// none and can be redone through the same API.
#[test]
fn orphan_cancellation_publishes_its_end_last_and_survives_a_crash() {
    for running in [
        Running::Tool,
        Running::DetachedTool,
        Running::LeftGroupTool,
        Running::ChildTool,
    ] {
        let case = crashed_running_case(running);
        let name = case.name();
        let (_temp, signals) = in_new_case_dir(case);
        run_first_life_until_crash(case, &signals);
        let store = signals.store();
        let paths = store.find_run_root(RUN).unwrap().unwrap();
        let controls = RuntimeControls::new(store.clone());
        assert_eq!(
            controls.request_cancel(RUN).unwrap().status,
            "cancel_requested"
        );

        let mut midway = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "routing_model_contracts::cancellation_matrix_contracts::cancellation_of_a_crashed_run_dies_midway",
                "--nocapture",
            ])
            .env(CANCEL_DIR_ENV, &signals.dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut status = None;
        wait_for(|| {
            status = midway.try_wait().unwrap();
            status.is_some()
        });
        if status.is_none() {
            let _ = midway.kill();
            let _ = midway.wait();
        }
        assert_eq!(
            status.and_then(|status| status.code()),
            Some(CRASH_EXIT_CODE),
            "{name}: must die with the tool told to stop"
        );
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().status,
            "cancel_requested",
            "{name}: terminal before settlement"
        );
        assert_eq!(
            store.read_result(&paths).unwrap().expect("result").state,
            "pending",
            "{name}: result published before settlement"
        );
        let canceled_events = || {
            store
                .read_raw_events(&paths)
                .unwrap()
                .iter()
                .filter(|event| event.payload.get("status") == Some(&json!("canceled")))
                .count()
        };
        assert_eq!(canceled_events(), 0, "{name}: event before settlement");
        assert!(
            tool_groups(&signals)
                .iter()
                .any(|group| group_running(group)),
            "{name}: the tool should still run"
        );

        // The first sight of the terminal state must find everything settled,
        // with no further wait for any of it.
        let observer_signals = signals.clone();
        let observer = std::thread::spawn(move || {
            let store = observer_signals.store();
            let paths = store.find_run_root(RUN).unwrap().unwrap();
            let saw_terminal =
                wait_for(|| store.read_run_record(&paths).unwrap().unwrap().status == "canceled");
            let children_settled = store
                .list_run_records()
                .unwrap()
                .iter()
                .filter(|record| record.parent_run_id.as_deref() == Some(RUN))
                .all(|record| record.status == "canceled");
            (
                saw_terminal,
                observer_signals.branch_settled(0),
                children_settled,
                tool_groups(&observer_signals)
                    .iter()
                    .all(|group| !group_running(group)),
                tool_ownership_entries(&observer_signals).is_empty(),
            )
        });
        let redone = bounded(&name, move || {
            RuntimeControls::new(signals.store())
                .mark_canceled(RUN, "no executor")
                .unwrap()
        });
        assert_eq!(redone.status, "canceled", "{name}");
        assert_eq!(
            observer.join().unwrap(),
            (true, true, true, true, true),
            "{name}"
        );
        assert_eq!(canceled_events(), 1, "{name}");
        assert_eq!(
            store.read_result(&paths).unwrap().expect("result").status,
            "canceled",
            "{name}"
        );
    }
}

/// With no resume at all, a crashed tool that ends on its own leaves nothing,
/// not even the process holding its group.
#[test]
fn an_unresumed_crash_leaves_nothing_once_its_tool_ends() {
    let case = crashed_running_case(Running::DetachedTool);
    let (_temp, signals) = in_new_case_dir(case);
    run_first_life_until_crash(case, &signals);
    let group = tool_groups(&signals).remove(0);
    assert!(group_running(&group), "the tool should still run");
    signals.raise("release");
    assert!(
        wait_for(|| !group_running(&group)),
        "group {group} still runs"
    );
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// How the first orphan cancellation of a crashed run fails.
#[derive(Debug, Clone, Copy)]
enum SettlementFailure {
    /// A canceled branch record cannot be written.
    BranchRecord,
    /// The ownership records cannot be listed.
    ToolRecords,
    /// The tool stops, but its ownership record cannot go.
    ToolRecordRemoval,
    /// The process table cannot be read: no `ps` or `lsof` on PATH.
    ProcessDiscovery,
}

/// A settlement that fails publishes nothing and keeps what a retry needs; the
/// same API then finishes it.
#[cfg(unix)]
#[test]
fn a_failed_orphan_settlement_publishes_nothing_and_can_be_redone() {
    for failure in [
        SettlementFailure::BranchRecord,
        SettlementFailure::ToolRecords,
        SettlementFailure::ToolRecordRemoval,
        SettlementFailure::ProcessDiscovery,
    ] {
        let case = crashed_running_case(Running::Tool);
        let (_temp, signals) = in_new_case_dir(case);
        run_first_life_until_crash(case, &signals);
        let store = signals.store();
        let paths = store.find_run_root(RUN).unwrap().unwrap();
        let controls = RuntimeControls::new(store.clone());
        let branches = signals.branches_dir().unwrap();
        let tools = paths.root.join("tool-processes");
        let group = tool_groups(&signals).remove(0);
        match failure {
            SettlementFailure::BranchRecord => {
                set_mode(&branches, 0o555);
                let error = controls.mark_canceled(RUN, "no executor");
                set_mode(&branches, 0o755);
                assert!(error.is_err(), "{failure:?}");
            }
            SettlementFailure::ToolRecords => {
                set_mode(&tools, 0o000);
                let error = controls.mark_canceled(RUN, "no executor");
                set_mode(&tools, 0o755);
                assert!(error.is_err(), "{failure:?}");
            }
            SettlementFailure::ToolRecordRemoval => {
                set_mode(&tools, 0o555);
                let error = controls.mark_canceled(RUN, "no executor");
                set_mode(&tools, 0o755);
                assert!(error.is_err(), "{failure:?}");
            }
            SettlementFailure::ProcessDiscovery => {
                let status = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "routing_model_contracts::cancellation_matrix_contracts::cancellation_of_a_crashed_run_dies_midway",
                        "--nocapture",
                    ])
                    .env(CANCEL_DIR_ENV, &signals.dir)
                    .env(CANCEL_EXPECTS_ERROR_ENV, "1")
                    .env("PATH", signals.path("no-tools-here"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap();
                assert_eq!(status.code(), Some(CRASH_EXIT_CODE), "{failure:?}");
            }
        }

        // Nothing terminal, and durable state a retry can work from.
        assert_eq!(
            store.read_run_record(&paths).unwrap().unwrap().status,
            "cancel_requested",
            "{failure:?}"
        );
        assert_eq!(
            store.read_result(&paths).unwrap().expect("result").state,
            "pending",
            "{failure:?}"
        );
        let canceled_events = || {
            store
                .read_raw_events(&paths)
                .unwrap()
                .iter()
                .filter(|event| event.payload.get("status") == Some(&json!("canceled")))
                .count()
        };
        assert_eq!(canceled_events(), 0, "{failure:?}");
        match failure {
            SettlementFailure::BranchRecord => {
                assert!(!signals.branch_settled(0), "{failure:?}");
            }
            SettlementFailure::ToolRecords | SettlementFailure::ProcessDiscovery => {
                assert!(group_running(&group), "{failure:?}: nothing stopped it");
                assert!(tools.join(&group).exists(), "{failure:?}: its record went");
            }
            SettlementFailure::ToolRecordRemoval => {
                assert!(!group_running(&group), "{failure:?}: it was told to stop");
                assert!(!tool_ownership_entries(&signals).is_empty(), "{failure:?}");
            }
        }

        let redone = bounded(&format!("{failure:?}"), {
            let store = store.clone();
            move || {
                RuntimeControls::new(store)
                    .mark_canceled(RUN, "no executor")
                    .unwrap()
            }
        });
        assert_eq!(redone.status, "canceled", "{failure:?}");
        assert!(!group_running(&group), "{failure:?}");
        assert!(signals.branch_settled(0), "{failure:?}");
        assert_eq!(
            tool_ownership_entries(&signals),
            Vec::<PathBuf>::new(),
            "{failure:?}"
        );
        assert_eq!(canceled_events(), 1, "{failure:?}");
        assert_eq!(
            store.read_result(&paths).unwrap().expect("result").status,
            "canceled",
            "{failure:?}"
        );
    }
}

/// A `ps` that holds the runtime where it reads a new anchor's identity, before
/// the record exists and before the tool may spawn, once `armed` has been
/// logged. Everything else goes to the real one.
fn write_registration_gate(signals: &Signals, armed: &str) -> PathBuf {
    let bin = signals.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let gate = bin.join("ps");
    let p = |name: &str| signals.path(name).display().to_string();
    std::fs::write(
        &gate,
        format!(
            "#!/bin/sh\n\
             if [ \"$1 $2\" = '-o lstart=' ] && grep -qx '{armed}' '{calls}' 2>/dev/null; then\n\
             echo \"$4\" > '{anchor}.tmp'; mv '{anchor}.tmp' '{anchor}'; touch '{at}'\n\
             i=0; while [ ! -f '{gate_release}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done\n\
             exit 1\nfi\nexec /bin/ps \"$@\"\n",
            calls = p("calls.log"),
            anchor = p("anchor"),
            at = p("at-registration"),
            gate_release = p("gate-release"),
        ),
    )
    .unwrap();
    #[cfg(unix)]
    set_mode(&gate, 0o755);
    bin
}

/// A crash in the window before a tool's first record: the executor has started
/// the anchor and nothing else. No tool ran, the anchor leaves on its own, and a
/// resume runs the tool once without repeating finished steps.
#[test]
fn a_crash_before_a_tool_is_registered_leaves_nothing_and_resumes() {
    for (running, armed) in [(Running::Tool, "pre"), (Running::ChildTool, "cpre")] {
        let case = crashed_running_case(running);
        let name = case.name();
        let (_temp, signals) = in_new_case_dir(case);
        let bin = write_registration_gate(&signals, armed);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.env(AT_REGISTRATION_ENV, "1").env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        run_first_life(case, &signals, command);

        assert!(signals.raised("at-registration"), "{name}");
        assert_eq!(signals.calls("work"), 0, "{name}: the tool must not start");
        assert_eq!(
            tool_ownership_entries(&signals),
            Vec::<PathBuf>::new(),
            "{name}: no record yet"
        );
        signals.raise("gate-release");
        let anchor = std::fs::read_to_string(signals.path("anchor")).unwrap();
        let anchor = anchor.trim();
        assert!(
            wait_for(|| !group_running(anchor)),
            "{name}: anchor {anchor} outlived its executor"
        );

        signals.raise("trigger");
        let release = signals.clone();
        std::thread::spawn(move || {
            if wait_for(|| release.calls("work") == 1) {
                release.raise("release");
            }
        });
        let (runner, store) = (runner(case, &signals), signals.store());
        let flow = flow(case, &signals);
        let result = bounded(&name, move || resume(runner, flow, store));
        assert_eq!(
            result.status, "completed",
            "{name}: {}",
            result.failure_reason
        );
        assert_eq!(result.outcome.as_deref(), Some("success"), "{name}");
        assert_eq!(signals.calls("pre"), 1, "{name}: pre ran again");
        assert_eq!(
            signals.calls("cpre"),
            usize::from(running.is_child()),
            "{name}: cpre ran again"
        );
        assert_eq!(signals.calls("work"), 1, "{name}: work invocations");
        assert_eq!(signals.calls("work-finished"), 1, "{name}");
        assert_eq!(signals.calls("synth"), 1, "{name}: synth invocations");
        assert_eq!(
            tool_ownership_entries(&signals),
            Vec::<PathBuf>::new(),
            "{name}"
        );
    }
}

fn real_tool(name: &str) -> String {
    let output = Command::new("sh")
        .args(["-c", &format!("command -v {name}")])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write_shim(signals: &Signals, name: &str, body: String) {
    let bin = signals.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join(name);
    std::fs::write(&shim, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    set_mode(&shim, 0o755);
}

/// This test binary, running only `isolated_tool_run`, with the case's shims
/// first on PATH: PATH is per process, so a shim cannot reach any other test.
fn isolated_run_command(signals: &Signals) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "routing_model_contracts::cancellation_matrix_contracts::isolated_tool_run",
            "--nocapture",
        ])
        .env(ISOLATED_DIR_ENV, &signals.dir)
        .env(
            "PATH",
            format!(
                "{}:{}",
                signals.path("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

const ISOLATED_CASE: Case = Case {
    fork: Fork::Strict,
    running: Running::Tool,
    stop: Stop::External,
    timing: Timing::MidExecution,
};

/// Child process entry point: runs the case's flow to its end and writes down
/// what the executor returned. A no-op in the ordinary test run.
#[test]
fn isolated_tool_run() {
    let Ok(dir) = std::env::var(ISOLATED_DIR_ENV) else {
        return;
    };
    let signals = Signals {
        dir: PathBuf::from(dir),
    };
    let flow: FlowDefinition =
        serde_json::from_str(&std::fs::read_to_string(signals.path("flow.json")).unwrap()).unwrap();
    let result = executor(runner(ISOLATED_CASE, &signals), &signals.store())
        .execute(fresh_request(flow, &signals))
        .expect("execute");
    std::fs::write(
        signals.path("result.json"),
        json!({"status": result.status, "reason": result.failure_reason}).to_string(),
    )
    .unwrap();
}

/// A tool with no durable owner never starts: whichever step of registration
/// fails, the invocation fails to launch and leaves no anchor behind.
#[test]
fn a_tool_that_cannot_be_registered_never_starts() {
    let record_anchor = |signals: &Signals| {
        format!(
            "[ \"$1 $2\" = '-o lstart=' ] && echo \"$4\" >> '{}'",
            signals.path("anchors").display()
        )
    };
    type Shims = fn(&Signals, String);
    let rows: [(&str, Shims); 3] = [
        ("identity", |signals, record| {
            write_shim(
                signals,
                "ps",
                format!("{record} && exit 1\nexec {} \"$@\"", real_tool("ps")),
            );
        }),
        ("output FIFOs", |signals, record| {
            write_shim(
                signals,
                "ps",
                format!("{record}\nexec {} \"$@\"", real_tool("ps")),
            );
            write_shim(signals, "mkfifo", "exit 1".to_string());
        }),
        ("record", |signals, record| {
            write_shim(
                signals,
                "ps",
                format!("{record}\nexec {} \"$@\"", real_tool("ps")),
            );
            write_shim(
                signals,
                "mkfifo",
                format!(
                    "{} \"$@\" && chmod 555 \"$(dirname \"$1\")\"",
                    real_tool("mkfifo")
                ),
            );
        }),
    ];
    for (step, shims) in rows {
        let (_temp, signals) = in_new_case_dir(ISOLATED_CASE);
        shims(&signals, record_anchor(&signals));
        let command = format!(
            "touch '{}'; ( sleep {} ) & exit 0",
            signals.path("started").display(),
            DEADLINE.as_secs() * 2
        );
        let flow = parse_flow(json!({
            "start": {"kind": "start", "routes": {"next": route("work")}},
            "work": tool(command, "done"),
            "done": exit("completed", "success"),
        }));
        std::fs::write(
            signals.path("flow.json"),
            serde_json::to_vec(&flow).unwrap(),
        )
        .unwrap();
        let mut run = isolated_run_command(&signals).spawn().unwrap();
        let mut status = None;
        wait_for(|| {
            status = run.try_wait().unwrap();
            status.is_some()
        });
        if status.is_none() {
            let _ = run.kill();
            let _ = run.wait();
        }
        assert!(status.is_some_and(|status| status.success()), "{step}");
        let result: Value =
            serde_json::from_str(&std::fs::read_to_string(signals.path("result.json")).unwrap())
                .unwrap();
        assert_eq!(result["status"], json!("failed"), "{step}: {result}");
        let reason = result["reason"].as_str().unwrap();
        assert!(
            reason.contains("could not be launched") && reason.contains("ownership"),
            "{step}: {reason}"
        );
        assert!(!signals.raised("started"), "{step}: the tool started");
        let anchors = std::fs::read_to_string(signals.path("anchors")).unwrap();
        assert!(!anchors.is_empty(), "{step}");
        for anchor in anchors.lines() {
            assert!(!group_running(anchor), "{step}: anchor {anchor} still runs");
        }
        // The record row's shim left the directory unwritable; let the temp
        // directory go.
        #[cfg(unix)]
        for entry in tool_ownership_entries(&signals) {
            set_mode(entry.parent().unwrap(), 0o755);
        }
    }
}

/// How a live cleanup with process discovery failing ends.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Recovery {
    /// Discovery comes back while the executor still runs.
    Restored,
    /// The executor dies first; orphan cancellation finishes after discovery
    /// comes back.
    ExecutorDies,
}

/// A stop whose process discovery fails keeps the tool's ownership and publishes
/// nothing. Once discovery returns, the descendant stops and the output settles
/// before the terminal state appears, in the live executor or from the record
/// after it dies.
#[test]
fn a_failed_live_cleanup_keeps_ownership_and_publishes_nothing_until_it_settles() {
    for (deadline, recovery) in [
        (true, Recovery::Restored),
        (false, Recovery::Restored),
        (true, Recovery::ExecutorDies),
        (false, Recovery::ExecutorDies),
    ] {
        let name = format!("deadline={deadline} {recovery:?}");
        let (_temp, signals) = in_new_case_dir(ISOLATED_CASE);
        let p = |name: &str| signals.path(name).display().to_string();
        signals.raise("discovery-down");
        write_shim(
            &signals,
            "ps",
            format!(
                "if [ \"$1\" = -A ] && [ -f '{down}' ]; then touch '{failed}'; exit 1; fi\n\
                 exec {ps} \"$@\"",
                down = p("discovery-down"),
                failed = p("discovery-failed"),
                ps = real_tool("ps"),
            ),
        );
        let command = format!(
            "set -m; ( exec 9<&-; echo descendant-output; \
             while ps -o stat= -p $$ | grep -qv Z; do sleep 0.02; done; touch '{started}'; \
             exec sleep {seconds} ) & echo $! >> '{holder}'; exit 0",
            started = p("started"),
            holder = p("holder"),
            seconds = DEADLINE.as_secs() * 2,
        );
        let mut flow = flow(ISOLATED_CASE, &signals);
        let work = flow.nodes.get_mut("work").unwrap();
        *work = serde_json::from_value(tool(command, "join")).unwrap();
        if deadline {
            work.runtime = Some(attractor_core::NodeRuntimeConfig {
                timeout: Some("2s".to_string()),
                ..Default::default()
            });
        }
        std::fs::write(
            signals.path("flow.json"),
            serde_json::to_vec(&flow).unwrap(),
        )
        .unwrap();
        let mut run = isolated_run_command(&signals).spawn().unwrap();
        let holders = || {
            std::fs::read_to_string(signals.path("holder"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let process_running = |pid: &str| {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&output.stdout);
            !stat.trim().is_empty() && !stat.trim().starts_with('Z')
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(wait_for(|| signals.raised("started")), "{name}: no tool");
            if !deadline {
                request_external_cancel(&signals);
            }
            assert!(
                wait_for(|| signals.raised("discovery-failed")),
                "{name}: the stop never reached process discovery"
            );

            // Unsettled: nothing terminal, nothing given up.
            let store = signals.store();
            let paths = store.find_run_root(RUN).unwrap().unwrap();
            let status = || store.read_run_record(&paths).unwrap().unwrap().status;
            assert_eq!(
                status(),
                if deadline {
                    "running"
                } else {
                    "cancel_requested"
                },
                "{name}"
            );
            assert_eq!(
                store.read_result(&paths).unwrap().expect("result").state,
                "pending",
                "{name}"
            );
            assert!(run.try_wait().unwrap().is_none(), "{name}: it returned");
            assert!(!signals.branch_settled(0), "{name}: settled too early");
            assert!(
                holders().iter().all(|pid| process_running(pid)),
                "{name}: the descendant ended without its stop"
            );
            let ownership = tool_ownership_entries(&signals);
            assert_eq!(ownership.len(), 3, "{name}: {ownership:?}");
            assert!(
                wait_for(
                    || store.read_raw_events(&paths).unwrap().iter().any(|event| {
                        event
                            .payload
                            .get("msg")
                            .and_then(Value::as_str)
                            .is_some_and(|msg| msg.contains("is not settled yet"))
                    })
                ),
                "{name}: nothing says why it waits"
            );

            // The first sight of the terminal state finds everything settled.
            let observer = {
                let signals = signals.clone();
                std::thread::spawn(move || {
                    let store = signals.store();
                    let paths = store.find_run_root(RUN).unwrap().unwrap();
                    let saw_terminal = wait_for(|| {
                        matches!(
                            store
                                .read_run_record(&paths)
                                .unwrap()
                                .unwrap()
                                .status
                                .as_str(),
                            "canceled" | "failed"
                        )
                    });
                    (
                        saw_terminal,
                        signals.branch_settled(0),
                        tool_ownership_entries(&signals).is_empty(),
                    )
                })
            };
            let holders_at_stop = holders();
            if recovery == Recovery::ExecutorDies {
                run.kill().unwrap();
                run.wait().unwrap();
                assert_eq!(tool_ownership_entries(&signals).len(), 3, "{name}");
                assert!(
                    holders_at_stop.iter().all(|pid| process_running(pid)),
                    "{name}: the crash must not have stopped it"
                );
            }
            std::fs::remove_file(signals.path("discovery-down")).unwrap();
            let expected = match recovery {
                Recovery::Restored => {
                    let mut exit = None;
                    wait_for(|| {
                        exit = run.try_wait().unwrap();
                        exit.is_some()
                    });
                    assert!(exit.is_some_and(|exit| exit.success()), "{name}: {exit:?}");
                    if deadline {
                        "failed"
                    } else {
                        "canceled"
                    }
                }
                Recovery::ExecutorDies => {
                    let store = signals.store();
                    bounded(&name, move || {
                        RuntimeControls::new(store)
                            .mark_canceled(RUN, "no executor")
                            .unwrap();
                    });
                    "canceled"
                }
            };
            assert_eq!(observer.join().unwrap(), (true, true, true), "{name}");
            assert_eq!(status(), expected, "{name}");
            for pid in holders() {
                assert!(
                    !process_running(&pid),
                    "{name}: descendant {pid} still runs"
                );
            }
            if recovery == Recovery::Restored {
                // The readers settled with what the descendant wrote.
                let log = std::fs::read_to_string(
                    paths.logs_dir().join("fork/executions/1-0/branches/0.json"),
                )
                .unwrap();
                assert!(log.contains("descendant-output"), "{name}: {log}");
            }
        }));
        // Protection for a failed case only.
        if let Err(payload) = outcome {
            let _ = run.kill();
            let _ = run.wait();
            for pid in holders() {
                let _ = Command::new("kill").args(["-9", &pid]).status();
            }
            std::panic::resume_unwind(payload);
        }
    }
}

/// What a tool's surviving descendant still holds of what the runtime gave it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Holds {
    Everything,
    OnlyOutput,
    OnlyRecord,
}

/// A tool's deadline and cancellation still work once its shell has exited, and
/// the runtime stops a descendant that left the group, by whatever it still
/// holds, and settles the output before it returns. `set -m` gives a background
/// job its own group where the shell supports it; elsewhere that row runs as the
/// in-group one. A record-only descendant redirected its output, so its shell
/// stays: with the shell gone and the output closed the tool has simply ended,
/// and a daemon it started on purpose stays.
#[test]
fn a_tool_settles_when_a_descendant_outlives_its_shell() {
    for (deadline, left_group, holds) in [
        (true, false, Holds::Everything),
        (true, true, Holds::Everything),
        (false, true, Holds::Everything),
        (true, true, Holds::OnlyOutput),
        (false, true, Holds::OnlyOutput),
        (true, true, Holds::OnlyRecord),
        (false, true, Holds::OnlyRecord),
    ] {
        let case = Case {
            fork: Fork::Strict,
            running: Running::Tool,
            stop: Stop::External,
            timing: Timing::MidExecution,
        };
        let name = format!("deadline={deadline} left_group={left_group} holds={holds:?}");
        let (_temp, signals) = in_new_case_dir(case);
        let p = |name: &str| signals.path(name).display().to_string();
        let (drop_handles, await_shell, shell_tail) = match holds {
            Holds::Everything => (
                "",
                "while ps -o stat= -p $$ | grep -qv Z; do sleep 0.02; done; ",
                "exit 0",
            ),
            Holds::OnlyOutput => (
                "exec 9<&-; ",
                "while ps -o stat= -p $$ | grep -qv Z; do sleep 0.02; done; ",
                "exit 0",
            ),
            Holds::OnlyRecord => ("exec >/dev/null 2>&1; ", "", "while :; do sleep 0.05; done"),
        };
        // Longer than every wait here, so a settlement that needs it to end
        // on its own fails the case.
        let command = format!(
            "{job_control}\
             ( {drop_handles}{await_shell}touch '{started}'; \
             exec sleep {seconds} ) & echo $! >> '{holder}'; {shell_tail}",
            job_control = if left_group { "set -m; " } else { "" },
            started = p("started"),
            holder = p("holder"),
            seconds = DEADLINE.as_secs() * 2,
        );
        let mut flow = flow(case, &signals);
        let work = flow.nodes.get_mut("work").unwrap();
        *work = serde_json::from_value(tool(command, "join")).unwrap();
        if deadline {
            work.runtime = Some(attractor_core::NodeRuntimeConfig {
                timeout: Some("2s".to_string()),
                ..Default::default()
            });
        } else {
            let conductor = signals.clone();
            std::thread::spawn(move || {
                if wait_for(|| conductor.raised("started")) {
                    request_external_cancel(&conductor);
                }
            });
        }
        // Asked once, right after the run returns: stopping it is the runtime's
        // job, not a later one's.
        let holder_running = |pid: &str| {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&output.stdout);
            !stat.trim().is_empty() && !stat.trim().starts_with('Z')
        };
        let (runner, store) = (runner(case, &signals), signals.store());
        let request = fresh_request(flow, &signals);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let result = bounded(&name, move || {
                executor(runner, &store).execute(request).expect("execute")
            });
            let affected = signals.branch_record(0);
            if deadline {
                assert_eq!(result.status, "failed", "{name}");
                assert!(
                    result.failure_reason.contains("timed out"),
                    "{name}: {}",
                    result.failure_reason
                );
                assert_eq!(affected["status"], json!("error"), "{name}: {affected}");
            } else {
                assert_eq!(result.status, "canceled", "{name}");
                assert_eq!(result.failure_reason, "aborted_by_user", "{name}");
                assert_eq!(affected["status"], json!("canceled"), "{name}: {affected}");
            }
            let holders = std::fs::read_to_string(signals.path("holder")).unwrap();
            assert!(!holders.is_empty(), "{name}");
            for pid in holders.lines() {
                assert!(!holder_running(pid), "{name}: descendant {pid} still runs");
            }
        }));
        // Protection for a failed case only: nothing is left after a passing one.
        if let Err(payload) = outcome {
            let holders = std::fs::read_to_string(signals.path("holder")).unwrap_or_default();
            for pid in holders.lines() {
                let _ = Command::new("kill").args(["-9", pid]).status();
            }
            std::panic::resume_unwind(payload);
        }
    }
}
