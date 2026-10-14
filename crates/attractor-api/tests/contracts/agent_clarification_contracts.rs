use attractor_api::{AttractorApiService, PipelineStartRequest};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Agent binaries come from process-wide variables, so scenarios that set
/// them run one at a time.
static AGENT_BIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[track_caller]
fn wait(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !check() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("project"),
        data_dir: root.join("home"),
        config_dir: root.join("home/config"),
        runtime_dir: root.join("home/runtime"),
        logs_dir: root.join("home/logs"),
        workspace_dir: root.join("home/workspace"),
        projects_dir: root.join("home/projects"),
        attractor_dir: root.join("home/attractor"),
        runs_dir: root.join("home/runs"),
        flows_dir: root.join("home/flows"),
        ui_dir: None,
        project_roots: vec![],
    }
}

#[test]
fn codex_clarification_continues_same_execution_and_keeps_durable_history() {
    clarification_scenario(Duration::from_millis(350));
}

#[test]
#[ignore = "Exercises the real five-minute Codex inactivity threshold"]
fn codex_human_wait_exceeds_inactivity_threshold() {
    clarification_scenario(Duration::from_secs(301));
}

fn clarification_scenario(human_wait: Duration) {
    let _env = AGENT_BIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut first_wait = Some(human_wait);
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.project_root).unwrap();
    let bin = temp.path().join("codex");
    fs::write(&bin, r#"#!/usr/bin/env python3
import json, sys, os, threading, time
round = 0
cwd = ''
def send(x):
    print(json.dumps(x), flush=True)
def ask():
    send({'id': 'request-'+str(round), 'method':'item/tool/requestUserInput', 'params':{
      'threadId':'thread', 'turnId':'turn', 'questions':[
        {'id':'choice', 'question':'Which audience?', 'options':[{'label':'Developers','description':'Technical readers'}]},
        {'id':'notes', 'question':'Any constraints?', 'options':[]}]}})
def watch_exit():
    while True:
        if cwd and os.path.exists(cwd+'/exit-agent'): os._exit(1)
        time.sleep(.05)
threading.Thread(target=watch_exit,daemon=True).start()
while True:
    line=sys.stdin.readline()
    if not line: break
    m=json.loads(line)
    if cwd:
        with open(cwd+'/rpc.jsonl','a') as f: f.write(json.dumps(m)+'\n')
    method=m.get('method')
    if method=='initialize': send({'id':m['id'],'result':{}})
    elif method=='thread/start':
        assert m['params']['config']['features.default_mode_request_user_input'] is True
        cwd=m['params']['cwd']
        send({'id':m['id'],'result':{'thread':{'id':'thread'}}})
    elif method=='turn/start':
        assert m['params']['collaborationMode']['mode']=='default'
        send({'id':m['id'],'result':{'turn':{'id':'turn'}}})
        ask()
    elif method=='turn/interrupt':
        send({'id':m['id'],'result':{}})
        send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'interrupted'}}})
    elif method is None and m.get('id','').startswith('request-'):
        assert m['result']['answers']['choice']['answers']==['Developers']
        assert m['result']['answers']['notes']['answers']==['Keep it short']
        round+=1
        if round==1: ask()
        else:
            send({'method':'item/completed','params':{'threadId':'thread','turnId':'turn','item':{'id':'msg','type':'agentMessage','text':'Done'}}})
            send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'completed'}}})
"#).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let old_bin = std::env::var_os("SPARK_CODEX_APP_SERVER_BIN");
    let old_runtime = std::env::var_os("ATTRACTOR_CODEX_RUNTIME_ROOT");
    std::env::set_var("SPARK_CODEX_APP_SERVER_BIN", &bin);
    std::env::set_var(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    );
    let start_request = PipelineStartRequest {
        run_id: Some("clarify".into()),
        working_directory: settings.project_root.to_string_lossy().into(),
        llm_provider: Some("codex".into()),
        model: Some("test-model".into()),
        flow_content: Some(
            r#"schema_version: "1"
id: clarify
nodes:
  start:
    kind: start
  work:
    kind: agent_task
    runtime:
      max_retries: 2
    config:
      kind: agent_task
      prompt: Resolve the intended audience
  done:
    kind: exit
edges:
- from: start
  to: work
- from: work
  to: done
"#
            .into(),
        ),
        ..Default::default()
    };
    let start = service.start_pipeline(start_request.clone());
    assert_eq!(start.status_code, 200, "{}", start.body);
    let mut old_ids = vec![];
    for _ in 0..2 {
        wait(|| {
            let bundle = attractor_runtime::RunStore::for_settings(&settings)
                .read_run_bundle("clarify")
                .unwrap()
                .unwrap();
            if bundle
                .record
                .as_ref()
                .is_some_and(|r| matches!(r.status.as_str(), "failed" | "completed"))
            {
                panic!("Run ended before questions: {:?}", bundle.raw_events);
            }

            service.list_pipeline_questions("clarify").body["questions"]
                .as_array()
                .is_some_and(|q| q.len() == 2)
        });
        // A new service represents a UI reload/disconnect, with no session state.
        let reloaded = AttractorApiService::new(settings.clone());
        let questions = reloaded.list_pipeline_questions("clarify").body["questions"]
            .as_array()
            .unwrap()
            .clone();
        for (index, q) in questions.iter().enumerate() {
            let id = q["question_id"].as_str().unwrap();
            assert!(!old_ids.contains(&id.to_string()));
            old_ids.push(id.to_string());
            assert_eq!(q["origin"], "agent_clarification");
            assert_eq!(q["attempt"], 0);
            let answer = if index == 0 {
                "Developers"
            } else {
                "Keep it short"
            };
            let path = format!("/attractor/pipelines/clarify/questions/{id}/answer");
            assert_eq!(
                reloaded
                    .dispatch("POST", &path, &json!({"selected_value":answer}).to_string())
                    .status_code,
                200
            );
            assert_ne!(
                reloaded
                    .dispatch(
                        "POST",
                        &path,
                        &json!({"selected_value":"overwrite"}).to_string()
                    )
                    .status_code,
                200
            );
            if index == 0 {
                thread::sleep(first_wait.take().unwrap_or(Duration::from_millis(350)));
                assert_eq!(
                    reloaded.list_pipeline_questions("clarify").body["questions"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
            }
        }
    }
    let store = attractor_runtime::RunStore::for_settings(&settings);
    wait(|| {
        store
            .read_run_bundle("clarify")
            .unwrap()
            .unwrap()
            .record
            .unwrap()
            .status
            == "completed"
    });
    let bundle = store.read_run_bundle("clarify").unwrap().unwrap();
    assert_eq!(
        bundle
            .raw_events
            .iter()
            .filter(|e| e.event_type == "InterviewCompleted")
            .count(),
        4
    );
    assert!(!bundle
        .raw_events
        .iter()
        .any(|e| e.event_type == "StageRetrying"));
    let rpc: Vec<Value> = fs::read_to_string(settings.project_root.join("rpc.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        rpc.iter().filter(|m| m["method"] == "turn/start").count(),
        1
    );
    assert_eq!(rpc.iter().filter(|m| m.get("result").is_some()).count(), 2);
    for control in ["cancel", "pause", "exit"] {
        let id = format!("clarify-{control}");
        let project = temp.path().join(&id);
        fs::create_dir_all(&project).unwrap();
        let start = service.start_pipeline(PipelineStartRequest {
            run_id: Some(id.clone()),
            working_directory: project.to_string_lossy().into(),
            ..start_request.clone()
        });
        assert_eq!(start.status_code, 200, "{}", start.body);
        wait(|| {
            service.list_pipeline_questions(&id).body["questions"]
                .as_array()
                .is_some_and(|q| q.len() == 2)
        });
        let questions = service.list_pipeline_questions(&id).body["questions"]
            .as_array()
            .unwrap()
            .clone();
        // Route this active execution as a child, and check the root question API.
        let mut child = store.read_run_bundle(&id).unwrap().unwrap();
        let mut record = child.record.take().unwrap();
        record.parent_run_id = Some("clarify".into());
        store.write_run_record(&child.paths, &record).unwrap();
        let root_questions = service.list_pipeline_questions("clarify").body["questions"]
            .as_array()
            .unwrap()
            .clone();
        assert!(root_questions.iter().any(|q| q["run_id"] == id));
        let question_id = questions[0]["question_id"].as_str().unwrap();
        let answer_path = format!("/attractor/pipelines/{id}/questions/{question_id}/answer");
        assert_eq!(
            service
                .dispatch("POST", &answer_path, r#"{"selected_value":"Developers"}"#)
                .status_code,
            200
        );
        match control {
            "exit" => fs::write(project.join("exit-agent"), "exit").unwrap(),
            _ => {
                // Persist the same control states the control APIs write.
                record.status = format!("{control}_requested");
                store.write_run_record(&child.paths, &record).unwrap();
            }
        }
        wait(|| {
            let status = store
                .read_run_bundle(&id)
                .unwrap()
                .unwrap()
                .record
                .unwrap()
                .status;
            matches!(status.as_str(), "failed" | "paused" | "canceled")
        });
        assert!(service.list_pipeline_questions(&id).body["questions"]
            .as_array()
            .unwrap()
            .is_empty());
        for q in &questions {
            assert!(!attractor_runtime::clarification::is_active(
                &child.paths,
                q
            ));
            let path = format!(
                "/attractor/pipelines/{id}/questions/{}/answer",
                q["question_id"].as_str().unwrap()
            );
            assert_ne!(
                service
                    .dispatch("POST", &path, r#"{"selected_value":"stale"}"#)
                    .status_code,
                200
            );
        }
        let historical = store.read_run_bundle(&id).unwrap().unwrap();
        if control != "exit" {
            assert!(!historical
                .raw_events
                .iter()
                .any(|e| e.event_type == "StageRetrying"));
        }
        assert_eq!(
            historical
                .raw_events
                .iter()
                .filter(|e| e.event_type == "human_gate")
                .count(),
            2
        );
        assert_eq!(
            historical
                .raw_events
                .iter()
                .filter(|e| e.event_type == "InterviewCompleted")
                .count(),
            1
        );
    }
    if let Some(value) = old_bin {
        std::env::set_var("SPARK_CODEX_APP_SERVER_BIN", value);
    } else {
        std::env::remove_var("SPARK_CODEX_APP_SERVER_BIN");
    }
    if let Some(value) = old_runtime {
        std::env::set_var("ATTRACTOR_CODEX_RUNTIME_ROOT", value);
    } else {
        std::env::remove_var("ATTRACTOR_CODEX_RUNTIME_ROOT");
    }
}

#[test]
fn dead_executor_questions_remain_historical_and_cannot_be_answered() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let mut record =
        attractor_core::RunRecord::new("orphan", settings.project_root.to_string_lossy());
    record.status = "waiting".into();
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    let directory = paths.root.join("clarifications");
    fs::create_dir_all(&directory).unwrap();
    let identity = "0123456789abcdef0123456789abcdef";
    let mut executor = Command::new("python3").args(["-c", "import fcntl,sys; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); print('ready',flush=True); sys.stdin.read()"])
        .arg(directory.join(format!("{identity}.active")))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut ready = String::new();
    BufReader::new(executor.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    for id in ["answered", "pending"] {
        let mut event = attractor_runtime::human_gate_pending_event(
            "orphan",
            id,
            "work",
            "flow",
            "Question?",
            None,
            vec![],
        );
        event
            .payload
            .insert("origin".into(), json!("agent_clarification"));
        event
            .payload
            .insert("clarification_id".into(), json!(identity));
        store.append_event(&paths, event).unwrap();
    }
    let service = AttractorApiService::new(settings.clone());
    assert_eq!(
        service.list_pipeline_questions("orphan").body["questions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        service
            .dispatch(
                "POST",
                "/attractor/pipelines/orphan/questions/answered/answer",
                r#"{"selected_value":"Accepted before disconnect"}"#
            )
            .status_code,
        200
    );
    executor.kill().unwrap();
    executor.wait().unwrap();
    let reloaded = AttractorApiService::new(settings);
    assert!(reloaded.list_pipeline_questions("orphan").body["questions"]
        .as_array()
        .unwrap()
        .is_empty());
    reloaded.recover_interrupted_runs();
    assert_ne!(
        reloaded
            .dispatch(
                "POST",
                "/attractor/pipelines/orphan/questions/pending/answer",
                r#"{"selected_value":"too late"}"#
            )
            .status_code,
        200
    );
    let history = store.read_raw_events(&paths).unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|e| e.event_type == "human_gate")
            .count(),
        2
    );
    assert!(history
        .iter()
        .any(|e| e.payload.get("answer") == Some(&json!("Accepted before disconnect"))));
}

const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, sys
def send(x):
    print(json.dumps(x), flush=True)
def read():
    line = sys.stdin.readline()
    return json.loads(line) if line else None
prompt = json.dumps(read())
send({'type':'system','subtype':'init','session_id':'s1','model':'m','tools':[],'capabilities':['interrupt_receipt_v1']})
if 'ASK' in prompt:
    assert 'AskUserQuestion' in prompt
    send({'type':'control_request','request_id':'q1','request':{'subtype':'can_use_tool','tool_name':'AskUserQuestion','input':{'questions':[
        {'question':'Which audience?','header':'Audience','options':[{'label':'Developers','description':'Technical readers'}],'multiSelect':False}]}}})
    answer = read()['response']['response']
    assert answer['behavior'] == 'allow', answer
    assert answer['updatedInput']['answers'] == {'Which audience?': 'Developers'}, answer
    send({'type':'result','subtype':'success','is_error':False,'result':'Done','session_id':'s1'})
elif 'LONG' in prompt:
    open('started','w').close()
    while True:
        m = read()
        if m is None: sys.exit(1)
        if m.get('type') == 'control_request' and m['request']['subtype'] == 'interrupt':
            send({'type':'control_response','response':{'subtype':'success','request_id':m['request_id'],'response':{'still_queued':[]}}})
            send({'type':'result','subtype':'error_during_execution','is_error':True,'terminal_reason':'aborted_streaming','session_id':'s1'})
            sys.exit(0)
"#;

/// A Claude CLI without `interrupt_receipt_v1`: control closes its stdin and
/// only killing the process stops it.
const FAKE_CLAUDE_WITHOUT_INTERRUPT: &str = r#"#!/usr/bin/env python3
import json, sys, time
print(json.dumps({'type':'system','subtype':'init','session_id':'s1','model':'m','tools':[]}), flush=True)
assert 'LONG' in sys.stdin.readline()
open('started','w').close()
time.sleep(600)
"#;

const FAKE_CODEX_LONG_TURN: &str = r#"#!/usr/bin/env python3
import json, sys
def send(x):
    print(json.dumps(x), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    method = m.get('method')
    if method == 'initialize': send({'id':m['id'],'result':{}})
    elif method == 'thread/start':
        cwd = m['params']['cwd']
        send({'id':m['id'],'result':{'thread':{'id':'thread'}}})
    elif method == 'turn/start':
        assert 'LONG' in json.dumps(m['params'])
        send({'id':m['id'],'result':{'turn':{'id':'turn'}}})
        open(cwd+'/started','w').close()
    elif method == 'turn/interrupt':
        send({'id':m['id'],'result':{}})
        send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'interrupted'}}})
"#;

/// A Codex app-server whose turn never stops: in a `*-cancel` project it
/// never answers `turn/interrupt`, otherwise it acknowledges the interrupt but
/// never completes the turn. It records its pid in `started`.
const FAKE_CODEX_UNRESPONSIVE: &str = r#"#!/usr/bin/env python3
import json, os, sys
def send(x):
    print(json.dumps(x), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    method = m.get('method')
    if method == 'initialize': send({'id':m['id'],'result':{}})
    elif method == 'thread/start':
        cwd = m['params']['cwd']
        send({'id':m['id'],'result':{'thread':{'id':'thread'}}})
    elif method == 'turn/start':
        send({'id':m['id'],'result':{'turn':{'id':'turn'}}})
        with open(cwd+'/started','w') as f: f.write(str(os.getpid()))
    elif method == 'turn/interrupt' and not cwd.endswith('cancel'):
        send({'id':m['id'],'result':{}})
"#;

fn install(temp: &Path, name: &str, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = temp.join(name);
    fs::write(&bin, script).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Starts a one-agent-node run in its own project directory.
fn start_node(
    service: &AttractorApiService,
    temp: &Path,
    id: &str,
    provider: &str,
    prompt: &str,
    runtime: &str,
) -> std::path::PathBuf {
    let project = temp.join(id);
    fs::create_dir_all(&project).unwrap();
    let start = service.start_pipeline(PipelineStartRequest {
        run_id: Some(id.into()),
        working_directory: project.to_string_lossy().into(),
        llm_provider: Some(provider.into()),
        model: Some("test-model".into()),
        flow_content: Some(format!(
            r#"schema_version: "1"
id: {id}
nodes:
  start:
    kind: start
  work:
    kind: agent_task
    runtime: {{ {runtime} }}
    config:
      kind: agent_task
      prompt: {prompt}
  done:
    kind: exit
edges:
- from: start
  to: work
- from: work
  to: done
"#
        )),
        ..Default::default()
    });
    assert_eq!(start.status_code, 200, "{}", start.body);
    project
}

fn run_status(settings: &SparkSettings, id: &str) -> String {
    attractor_runtime::RunStore::for_settings(settings)
        .read_run_bundle(id)
        .unwrap()
        .and_then(|bundle| bundle.record)
        .map(|record| record.status)
        .unwrap_or_default()
}

fn stage_failures(settings: &SparkSettings, id: &str) -> Vec<String> {
    let store = attractor_runtime::RunStore::for_settings(settings);
    store
        .read_run_bundle(id)
        .unwrap()
        .unwrap()
        .raw_events
        .iter()
        .filter(|e| e.event_type == "StageFailed")
        .map(|e| e.payload["error"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Cancel interrupts a node that would otherwise never finish, and a node
/// `timeout` interrupts and fails it.
fn assert_cancel_and_timeout(
    service: &AttractorApiService,
    settings: &SparkSettings,
    temp: &Path,
    provider: &str,
) {
    let id = format!("{provider}-cancel");
    let project = start_node(service, temp, &id, provider, "LONG work", "");
    wait(|| project.join("started").exists());
    let path = format!("/attractor/pipelines/{id}/cancel");
    assert_eq!(service.dispatch("POST", &path, "{}").status_code, 200);
    wait(|| run_status(settings, &id) == "canceled");
    assert_eq!(stage_failures(settings, &id), Vec::<String>::new());

    let id = format!("{provider}-timeout");
    start_node(
        service,
        temp,
        &id,
        provider,
        "LONG work",
        "timeout: 1s, max_retries: 0",
    );
    wait(|| run_status(settings, &id) == "failed");
    let failures = stage_failures(settings, &id);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].contains("timed out after 1 seconds"),
        "{failures:?}"
    );
}

#[test]
fn claude_code_node_questions_cancel_and_timeout() {
    let _env = AGENT_BIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.project_root).unwrap();
    let bin = install(temp.path(), "claude", FAKE_CLAUDE);
    let old_bin = std::env::var_os("SPARK_CLAUDE_CODE_BIN");
    std::env::set_var("SPARK_CLAUDE_CODE_BIN", &bin);
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    );

    start_node(
        &service,
        temp.path(),
        "claude-ask",
        "claude-code",
        "ASK about the audience",
        "",
    );
    wait(|| {
        assert_ne!(run_status(&settings, "claude-ask"), "failed");
        service.list_pipeline_questions("claude-ask").body["questions"]
            .as_array()
            .is_some_and(|q| q.len() == 1)
    });
    assert_eq!(run_status(&settings, "claude-ask"), "waiting");
    let question = service.list_pipeline_questions("claude-ask").body["questions"][0].clone();
    assert_eq!(question["origin"], "agent_clarification");
    assert_eq!(question["options"][0]["label"], "Developers");
    let path = format!(
        "/attractor/pipelines/claude-ask/questions/{}/answer",
        question["question_id"].as_str().unwrap()
    );
    assert_eq!(
        service
            .dispatch("POST", &path, r#"{"selected_value":"Developers"}"#)
            .status_code,
        200
    );
    wait(|| {
        let status = run_status(&settings, "claude-ask");
        assert_ne!(status, "failed");
        status == "completed"
    });

    assert_cancel_and_timeout(&service, &settings, temp.path(), "claude-code");
    match old_bin {
        Some(value) => std::env::set_var("SPARK_CLAUDE_CODE_BIN", value),
        None => std::env::remove_var("SPARK_CLAUDE_CODE_BIN"),
    }
}

#[test]
fn claude_code_node_without_interrupt_is_terminated_on_cancel_and_timeout() {
    let _env = AGENT_BIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.project_root).unwrap();
    let bin = install(temp.path(), "claude", FAKE_CLAUDE_WITHOUT_INTERRUPT);
    let old_bin = std::env::var_os("SPARK_CLAUDE_CODE_BIN");
    std::env::set_var("SPARK_CLAUDE_CODE_BIN", &bin);
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    );
    assert_cancel_and_timeout(&service, &settings, temp.path(), "claude-code");
    match old_bin {
        Some(value) => std::env::set_var("SPARK_CLAUDE_CODE_BIN", value),
        None => std::env::remove_var("SPARK_CLAUDE_CODE_BIN"),
    }
}

#[test]
fn codex_node_that_does_not_stop_is_terminated_on_cancel_and_timeout() {
    let _env = AGENT_BIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.project_root).unwrap();
    let bin = install(temp.path(), "codex", FAKE_CODEX_UNRESPONSIVE);
    let old_bin = std::env::var_os("SPARK_CODEX_APP_SERVER_BIN");
    let old_runtime = std::env::var_os("ATTRACTOR_CODEX_RUNTIME_ROOT");
    std::env::set_var("SPARK_CODEX_APP_SERVER_BIN", &bin);
    std::env::set_var(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    );
    // `wait` bounds each stop at 15s, past the 10s interrupt grace.
    assert_cancel_and_timeout(&service, &settings, temp.path(), "codex");
    for id in ["codex-cancel", "codex-timeout"] {
        let pid = fs::read_to_string(temp.path().join(id).join("started")).unwrap();
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "{id} app-server {pid} still running");
    }
    for (key, value) in [
        ("SPARK_CODEX_APP_SERVER_BIN", old_bin),
        ("ATTRACTOR_CODEX_RUNTIME_ROOT", old_runtime),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn codex_node_cancel_and_timeout_interrupt_the_turn() {
    let _env = AGENT_BIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.project_root).unwrap();
    let bin = install(temp.path(), "codex", FAKE_CODEX_LONG_TURN);
    let old_bin = std::env::var_os("SPARK_CODEX_APP_SERVER_BIN");
    let old_runtime = std::env::var_os("ATTRACTOR_CODEX_RUNTIME_ROOT");
    std::env::set_var("SPARK_CODEX_APP_SERVER_BIN", &bin);
    std::env::set_var(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    );
    assert_cancel_and_timeout(&service, &settings, temp.path(), "codex");
    for (key, value) in [
        ("SPARK_CODEX_APP_SERVER_BIN", old_bin),
        ("ATTRACTOR_CODEX_RUNTIME_ROOT", old_runtime),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
