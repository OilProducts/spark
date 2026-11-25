//! Whole-task handoff between two real `spark-server` processes: A delegates
//! flows and conversations to B through a link, and each delegation still
//! ends with exactly one result in A's owner conversation and at most one
//! launch on B, whichever server stops, loses its link or is canceled on the
//! way.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const OWNER: &str = "chat-owner";

/// One `spark-server serve` on a fixed port, so a restart keeps its origin.
struct Server {
    root: PathBuf,
    port: u16,
    codex: PathBuf,
    /// An agent that fails at once.
    agent: PathBuf,
    child: Option<Child>,
}

/// The fake Codex app-server: each turn asks two questions, ends once they
/// are answered, and ends interrupted when stopped.
fn fake_codex() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().join(format!(
        "spark-agent-fake-codex-app-server{}",
        std::env::consts::EXE_SUFFIX
    ))
}

impl Server {
    /// A server whose agents fail at once, or run as `codex`.
    fn new(root: PathBuf, codex: Option<PathBuf>) -> Self {
        fs::create_dir_all(root.join("ui")).unwrap();
        fs::write(root.join("ui/index.html"), "<!doctype html>\n").unwrap();
        let agent =
            crate::server_shell_contracts::fake_agent(&root, "agent", "exit 1", "exit /b 1");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut server = Self {
            codex: codex.unwrap_or_else(|| agent.clone()),
            agent,
            root,
            port,
            child: None,
        };
        server.start();
        server
    }
    fn data_dir(&self) -> PathBuf {
        self.root.join("spark-home")
    }
    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    fn start(&mut self) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_spark-server"))
            .env("HOME", &self.root)
            .env_remove("SPARK_HOME")
            .env_remove("SPARK_API_BASE_URL")
            .env("SPARK_CODEX_APP_SERVER_BIN", &self.codex)
            .env(
                "SPARK_FAKE_CODEX_APP_SERVER_MODE",
                "interruptible-questions",
            )
            .env("SPARK_CLAUDE_CODE_BIN", &self.agent)
            .args(["serve", "--host", "127.0.0.1", "--port"])
            .arg(self.port.to_string())
            .arg("--data-dir")
            .arg(self.data_dir())
            .arg("--ui-dir")
            .arg(self.root.join("ui"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn spark-server");
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (listening, printed) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if line.starts_with("Spark server listening") {
                    let _ = listening.send(());
                }
            }
        });
        if let Err(error) = printed.recv_timeout(Duration::from_secs(180)) {
            let _ = child.kill();
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            panic!("spark-server did not start ({error}): {stderr}");
        }
        self.child = Some(child);
    }
    /// Ends the server the hard way, as a crash or power loss would.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn environment_id(&self) -> String {
        reqwest::blocking::get(format!("{}/environment", self.origin()))
            .unwrap()
            .json::<Value>()
            .unwrap()["environment_id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let client = reqwest::blocking::Client::new();
        let url = format!("{}/workspace/api{path}", self.origin());
        let request = match method {
            "GET" => client.get(url),
            "DELETE" => client.delete(url),
            _ => client.post(url).json(&body.unwrap_or(json!({}))),
        };
        let response = request.send().expect("request");
        let status = response.status();
        let body: Value = response.json().unwrap_or(Value::Null);
        assert!(status.is_success(), "{method} {path}: {status} {body}");
        body
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A flow whose one tool step waits until `release` exists in the project.
fn hold_flow(release: &str) -> Value {
    json!({
        "schema_version": "2",
        "id": release,
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"},
                "routes": {"next": {"to": "work", "description": "Start the work."}}},
            "work": {"kind": "tool",
                "config": {"kind": "tool", "command": format!("while [ ! -e {release} ]; do sleep 0.1; done"),
                    "exit_codes": {"0": "done"}},
                "routes": {"done": {"to": "done", "description": "The work finished."}}},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
        }
    })
}

fn gate_flow() -> Value {
    json!({
        "schema_version": "2",
        "id": "gate",
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"},
                "routes": {"next": {"to": "review", "description": "Ask for approval."}}},
            "review": {"kind": "human_gate", "config": {"kind": "human_gate", "prompt": "Ship it?"},
                "routes": {
                    "approve": {"to": "done", "description": "Ship it.", "label": "Approve"},
                    "reject": {"to": "rejected", "description": "Stop.", "label": "Reject"}}},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}},
            "rejected": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "rejected", "outcome": "failure"}}}
        }
    })
}

struct Pair {
    a: Server,
    b: Server,
    b_id: String,
    b_project: PathBuf,
    handle: String,
}

impl Pair {
    fn new(root: &Path) -> Self {
        // A's owner conversation exists before A starts.
        let a_project = root.join("a/project");
        fs::create_dir_all(&a_project).unwrap();
        {
            let store = spark_storage::ConversationStore::open(root.join("a/spark-home")).unwrap();
            let conversations = spark_storage::ConversationRepository::new(store);
            conversations
                .touch_conversation(OWNER, a_project.to_str().unwrap())
                .unwrap();
        }
        let handle = {
            let store = spark_storage::ConversationStore::open(root.join("a/spark-home")).unwrap();
            spark_storage::ConversationRepository::new(store)
                .find_conversation_handle(OWNER)
                .unwrap()
                .unwrap()
        };
        let b_project = root.join("b/project");
        let flows = b_project.join(".spark/definitions/flows/tests");
        for (name, flow) in [
            ("gate", gate_flow()),
            ("hold-1", hold_flow("hold-1")),
            ("hold-3", hold_flow("hold-3")),
            ("hold-4", hold_flow("hold-4")),
            ("hold-5", hold_flow("hold-5")),
            ("hold-6", hold_flow("hold-6")),
            ("hold-7", hold_flow("hold-7")),
        ] {
            let flow =
                attractor_dsl::validated_flow(serde_json::from_value(flow).unwrap()).unwrap();
            attractor_dsl::write_flow_dir(&flows.join(name), &flow).unwrap();
        }
        let b = Server::new(root.join("b"), Some(fake_codex()));
        b.call(
            "POST",
            "/projects/register",
            Some(json!({"project_path": b_project})),
        );
        let b_id = b.environment_id();
        let pair = Self {
            a: Server::new(root.join("a"), None),
            b,
            b_id,
            b_project,
            handle,
        };
        pair.link();
        pair
    }
    fn link(&self) {
        let code = self.b.call("POST", "/environment/pairing-codes", None)["code"].clone();
        self.a.call(
            "POST",
            "/environments/links",
            Some(json!({"origin": self.b.origin(), "expected_id": self.b_id, "code": code})),
        );
    }
    fn start(&self, key: &str, flow: &str) -> String {
        self.delegate(
            key,
            json!({
                "kind": "flow",
                "flow_name": format!("tests/{flow}"),
                "summary": format!("Delegated {key}"),
            }),
        )
    }
    fn start_conversation(&self, key: &str) -> String {
        self.delegate(
            key,
            json!({"kind": "conversation", "message": "Ask me two things."}),
        )
    }
    fn delegate(&self, key: &str, mut task: Value) -> String {
        task["environment_id"] = json!(self.b_id);
        task["project_path"] = json!(self.b_project);
        task["request_key"] = json!(key);
        task["conversation_handle"] = json!(self.handle);
        let record = self.a.call("POST", "/delegations", Some(task));
        record["delegation_id"].as_str().unwrap().to_string()
    }
    /// Waits until A sees `id`'s first open question, and returns it.
    fn wait_question(&self, id: &str) -> Value {
        let mut question = Value::Null;
        wait_for(&format!("{id}'s question on A"), || {
            question = self.get(id)["last_state"]["questions"][0].clone();
            question.is_object()
        });
        question
    }
    fn get(&self, id: &str) -> Value {
        self.a.call("GET", &format!("/delegations/{id}"), None)
    }
    fn wait_phase(&self, id: &str, phase: &str) -> Value {
        let mut record = Value::Null;
        wait_for(&format!("{id} to reach {phase} on A"), || {
            record = self.get(id);
            record["phase"] == phase
        });
        record
    }
    /// B's own view of `id`.
    fn on_b(&self, id: &str) -> Value {
        self.b
            .call("GET", &format!("/delegations/incoming/{id}"), None)
    }
    /// Waits until A follows `id` and B's run for it is running.
    fn wait_running(&self, id: &str) {
        self.wait_phase(id, "accepted");
        wait_for(&format!("{id}'s run to start on B"), || {
            self.on_b(id)["status"] == "running"
        });
    }
    fn release(&self, flow: &str) {
        fs::write(self.b_project.join(flow), "").unwrap();
    }
}

#[test]
fn every_delegation_seals_once_whatever_stops_on_the_way() {
    let temp = tempfile::tempdir().unwrap();
    let root = spark_common::paths::canonicalize_clean(temp.path()).unwrap();
    let mut pair = Pair::new(&root);

    // B is down when A records two tasks and cancels one of them, and a third
    // was admitted on B, which stopped before it launched it.
    pair.b.kill();
    let sent_late = pair.start("sent-late", "hold-1");
    let canceled_unsent = pair.start("canceled-unsent", "hold-1");
    pair.a.call(
        "POST",
        &format!("/delegations/{canceled_unsent}/cancel"),
        None,
    );
    let unlaunched = pair.start("unlaunched", "hold-3");
    let record = pair.get(&unlaunched);
    let unlaunched_target = "00000000-0000-4000-8000-000000000003";
    spark_storage::environment::EnvironmentRepository::new(pair.b.data_dir())
        .admit_in(&spark_storage::environment::DelegationIn {
            delegation_id: unlaunched.clone(),
            origin_environment_id: pair.a.environment_id(),
            project_path: record["project_path"].as_str().unwrap().into(),
            kind: "flow".into(),
            request: record["request"].clone(),
            request_sha256: record["request_sha256"].as_str().unwrap().into(),
            target_kind: "run".into(),
            target_id: unlaunched_target.into(),
            phase: "accepted".into(),
            result: None,
            created_at: spark_storage::environment::now(),
            sealed_at: None,
            canceled_at: None,
        })
        .unwrap();
    pair.b.start();
    assert_eq!(
        pair.wait_phase(&canceled_unsent, "sealed")["result"]["status"],
        "canceled"
    );
    pair.wait_running(&sent_late);
    pair.wait_running(&unlaunched);
    assert_eq!(pair.get(&unlaunched)["target_id"], unlaunched_target);

    // B finishes while A is down; A collects the outcome when it returns.
    pair.a.kill();
    pair.release("hold-1");
    wait_for("B to seal sent-late", || {
        pair.on_b(&sent_late)["phase"] == "sealed"
    });
    pair.a.start();
    assert_eq!(
        pair.wait_phase(&sent_late, "sealed")["result"]["status"],
        "completed"
    );

    // B stops mid-run, and its recovery settles the run; A stops and returns
    // before it collects the outcome, then follows the task to its end.
    pair.b.kill();
    pair.b.start();
    pair.a.kill();
    pair.a.start();
    pair.release("hold-3");
    pair.wait_phase(&unlaunched, "sealed");

    // The link is cut mid-task: A pauses, and resumes when it is linked again.
    let unlinked = pair.start("unlinked", "hold-4");
    pair.wait_running(&unlinked);
    pair.a.call(
        "DELETE",
        &format!("/environments/links/{}", pair.b_id),
        None,
    );
    pair.release("hold-4");
    wait_for("B to seal unlinked", || {
        pair.on_b(&unlinked)["phase"] == "sealed"
    });
    let paused = pair.get(&unlinked);
    assert!(paused["paused"].is_string(), "{paused}");
    assert_eq!(paused["phase"], "accepted");
    pair.link();
    assert_eq!(
        pair.wait_phase(&unlinked, "sealed")["result"]["status"],
        "completed"
    );

    // Cancels: of a running task, racing its completion, and of one B has
    // not admitted yet, which B never launches unless the follower's tick
    // fell between its start and its cancel.
    let running = pair.start("canceled-running", "hold-5");
    let racing = pair.start("canceled-racing", "hold-6");
    pair.wait_running(&running);
    pair.wait_running(&racing);
    pair.a
        .call("POST", &format!("/delegations/{running}/cancel"), None);
    pair.release("hold-6");
    pair.a
        .call("POST", &format!("/delegations/{racing}/cancel"), None);
    let unadmitted = pair.start("canceled-unadmitted", "hold-7");
    pair.a
        .call("POST", &format!("/delegations/{unadmitted}/cancel"), None);
    assert_eq!(
        pair.wait_phase(&running, "sealed")["result"]["status"],
        "canceled"
    );
    let raced = pair.wait_phase(&racing, "sealed")["result"]["status"].clone();
    assert!(raced == "completed" || raced == "canceled", "{raced}");
    assert_eq!(
        pair.wait_phase(&unadmitted, "sealed")["result"]["status"],
        "canceled"
    );

    // A question on B reaches A's owner and is answered through A.
    let asking = pair.start("asking", "gate");
    let question = pair.wait_question(&asking);
    let question_id = question["question_id"].as_str().unwrap();
    pair.a.call(
        "POST",
        &format!("/delegations/{asking}/questions/{question_id}/answer"),
        Some(json!({"selected_value": "approve"})),
    );
    assert_eq!(
        pair.wait_phase(&asking, "sealed")["result"]["status"],
        "completed"
    );

    // A conversation on B asks two questions in one request, and A answers
    // both. Another is canceled while it waits for its answers, and seals.
    let answered = pair.start_conversation("answered");
    let question = pair.wait_question(&answered);
    assert_eq!(question["questions"].as_array().unwrap().len(), 2);
    let question_id = question["question_id"].as_str().unwrap();
    pair.a.call(
        "POST",
        &format!("/delegations/{answered}/questions/{question_id}/answer"),
        Some(json!({"answers": {"choice": "A", "notes": "Nothing else."}})),
    );
    let sealed = pair.wait_phase(&answered, "sealed");
    assert_eq!(sealed["result"]["status"], "complete", "{sealed}");
    let waiting = pair.start_conversation("canceled-waiting");
    pair.wait_question(&waiting);
    pair.a
        .call("POST", &format!("/delegations/{waiting}/cancel"), None);
    let sealed = pair.wait_phase(&waiting, "sealed");
    assert_eq!(sealed["result"]["status"], "canceled", "{sealed}");

    pair.a.kill();
    pair.b.kill();
    let store = spark_storage::ConversationStore::open(pair.a.data_dir()).unwrap();
    let events = spark_storage::ConversationRepository::new(store)
        .inbox_events(OWNER)
        .unwrap();
    let runs = attractor_runtime::RunStore::for_runs_root(attractor_runtime::RunsRoot::Home(
        pair.b.data_dir(),
    ))
    .list_run_records()
    .unwrap();
    for (id, launches) in [
        (&sent_late, 1..=1),
        (&canceled_unsent, 0..=0),
        (&unlaunched, 1..=1),
        (&unlinked, 1..=1),
        (&running, 1..=1),
        (&racing, 1..=1),
        (&unadmitted, 0..=1),
        (&asking, 1..=1),
        (&answered, 0..=0),
        (&waiting, 0..=0),
    ] {
        let results = events
            .iter()
            .filter(|event| event.kind == "delegation.result" && &event.source == id)
            .count();
        assert_eq!(results, 1, "{id}: {events:?}");
        let launched: Vec<_> = runs
            .iter()
            .filter(|run| {
                run.launch_context
                    .as_ref()
                    .and_then(|context| context.get("context.spark_delegation"))
                    .is_some_and(|provenance| provenance["delegation_id"] == id.as_str())
            })
            .collect();
        assert!(launches.contains(&launched.len()), "{id}: {launched:?}");
    }
    assert!(runs.iter().any(|run| run.run_id == unlaunched_target));
    assert!(events
        .iter()
        .any(|event| event.kind == "delegation.question" && event.source == asking));
}

/// A native flow on A delegates from its tool step with the `spark` CLI, which
/// reaches A although the CLI's saved target is another server.
#[test]
fn a_flow_tool_delegates_through_its_own_server() {
    let temp = tempfile::tempdir().unwrap();
    let root = spark_common::paths::canonicalize_clean(temp.path()).unwrap();
    let pair = Pair::new(&root);
    let saved = root.join("a/.spark/config");
    fs::create_dir_all(&saved).unwrap();
    fs::write(
        saved.join("spark.toml"),
        format!(
            "[connections]\nclient_api_base_url = \"{}\"\n",
            pair.b.origin()
        ),
    )
    .unwrap();
    let a_project = root.join("a/project");
    let flow = json!({
        "schema_version": "2",
        "id": "handoff",
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"},
                "routes": {"next": {"to": "handoff", "description": "Hand the work over."}}},
            "handoff": {"kind": "tool",
                "config": {"kind": "tool",
                    "env_map": {"SPARK_RUN_ID": "internal.run_id"},
                    "command": format!(
                        "spark delegation start --run \"$SPARK_RUN_ID\" --request-key handoff \
                         --environment {} --target-project '{}' --kind flow --flow tests/hold-1 --summary Handoff",
                        pair.b_id,
                        pair.b_project.display()
                    ),
                    "exit_codes": {"0": "done"}},
                "routes": {"done": {"to": "done", "description": "B has the work."}}},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
        }
    });
    attractor_dsl::write_flow_dir(
        &a_project.join(".spark/definitions/flows/tests/handoff"),
        &attractor_dsl::validated_flow(serde_json::from_value(flow).unwrap()).unwrap(),
    )
    .unwrap();
    pair.a.call(
        "POST",
        "/projects/register",
        Some(json!({"project_path": a_project})),
    );
    let launched = reqwest::blocking::Client::new()
        .post(format!("{}/attractor/pipelines", pair.a.origin()))
        .json(&json!({
            "run_id": "handoff-run",
            "flow_name": "tests/handoff",
            "working_directory": a_project,
            "project_path": a_project,
        }))
        .send()
        .unwrap();
    assert!(
        launched.status().is_success(),
        "{}",
        launched.text().unwrap()
    );

    let runs = attractor_runtime::RunStore::for_runs_root(attractor_runtime::RunsRoot::Home(
        pair.a.data_dir(),
    ));
    let mut status = String::new();
    wait_for("the handoff run to end on A", || {
        status = runs
            .list_run_records()
            .unwrap()
            .into_iter()
            .find(|run| run.run_id == "handoff-run")
            .map(|run| run.status)
            .unwrap_or_default();
        ["completed", "failed"].contains(&status.as_str())
    });
    assert_eq!(status, "completed");
    let delegation = spark_storage::environment::EnvironmentRepository::new(pair.a.data_dir())
        .out_by_key("run:handoff-run", "handoff")
        .unwrap()
        .expect("A records the run's delegation");
    assert_eq!(delegation.environment_id, pair.b_id);
    pair.wait_running(&delegation.delegation_id);
}
