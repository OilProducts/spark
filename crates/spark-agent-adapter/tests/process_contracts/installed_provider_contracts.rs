//! Attachment input, mid-turn steering and interrupt against the installed `codex` and `claude` binaries.
//! These spend real model turns and need signed-in providers, so they are
//! opt-in:
//! `cargo test --workspace --all-features --test process_contracts installed_ -- --ignored`

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentAttachment, AgentTurnBackend, AgentTurnRequest, CodexAppServerBackend,
    RustLlmAgentTurnBackend, SessionSteeringHandle, AGENT_ATTACHMENTS_METADATA_KEY,
};
use spark_common::debug::{CODEX_JSONRPC_TRACE_PATH_METADATA_KEY, ENV_SPARK_DEBUG_CODEX_JSONRPC};
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind};
use unified_llm_adapter::Client;

use super::test_support::ENV_LOCK;

/// A 16x16 solid red PNG.
const RED_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVR4nGP4z8BAEiJN9aiGUQ1DSgMAkPn/Afnh+ngAAAAASUVORK5CYII=";
const PROMPT: &str = "Reply only as colour,word in lowercase. colour is the single colour filling the attached image, or none if no image reached you. word is the one word in the attached text file, or none if you cannot read it.";

fn attachments(dir: &std::path::Path) -> Value {
    let image = base64::engine::general_purpose::STANDARD
        .decode(RED_PNG_BASE64)
        .expect("png");
    let write = |name: &str, media_type: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).expect("attachment bytes");
        AgentAttachment {
            id: format!("att-{name}"),
            filename: name.to_string(),
            media_type: media_type.to_string(),
            size_bytes: bytes.len() as u64,
            path: path.to_string_lossy().to_string(),
        }
    };
    json!([
        write("swatch.png", "image/png", &image),
        write("word.txt", "text/plain", b"pineapple"),
    ])
}

fn request(dir: &std::path::Path, provider: &str) -> AgentTurnRequest {
    AgentTurnRequest {
        conversation_id: format!("installed-{provider}"),
        project_path: dir.to_string_lossy().to_string(),
        prompt: PROMPT.to_string(),
        history: Vec::new(),
        provider: Some(provider.to_string()),
        model: None,
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: None,
        metadata: BTreeMap::from([(AGENT_ATTACHMENTS_METADATA_KEY.to_string(), attachments(dir))]),
    }
}

/// Delivery, not perception: the file word is exact, and the image only has
/// to have reached the model (shade names vary between runs).
fn assert_answer(answer: Option<String>) {
    let answer = answer.unwrap_or_default().trim().to_lowercase();
    let (colour, word) = answer
        .trim_end_matches('.')
        .split_once(',')
        .unwrap_or_else(|| panic!("unexpected answer: {answer}"));
    let colour = colour.trim();
    assert!(
        !colour.is_empty() && colour != "none",
        "image not delivered: {answer}"
    );
    assert_eq!(word.trim(), "pineapple", "file not delivered: {answer}");
}

#[test]
#[ignore = "runs the installed codex app-server against a signed-in account"]
fn installed_codex_accepts_local_images_and_file_references() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    std::env::set_var(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    let temp = tempfile::tempdir().expect("tempdir");
    let trace = temp.path().join("codex-trace.jsonl");
    let mut request = request(temp.path(), "codex");
    request.metadata.insert(
        CODEX_JSONRPC_TRACE_PATH_METADATA_KEY.to_string(),
        json!(trace.to_string_lossy()),
    );

    let output = CodexAppServerBackend::new()
        .run_agent_turn(request)
        .expect("installed codex turn");
    std::env::remove_var(ENV_SPARK_DEBUG_CODEX_JSONRPC);

    // The request Spark sent matches the installed binary's own schema.
    let schema_dir = temp.path().join("schema");
    let status = Command::new("codex")
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(&schema_dir)
        .status()
        .expect("codex app-server generate-json-schema");
    assert!(status.success());
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(schema_dir.join("v2/TurnStartParams.json")).expect("schema"),
    )
    .expect("schema json");
    let validator = jsonschema::validator_for(&schema).expect("validator");
    let turn_starts = fs::read_to_string(&trace)
        .expect("trace")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("trace line"))
        .filter(|record| record["direction"] == "outgoing")
        .map(|record| serde_json::from_str::<Value>(record["line"].as_str().unwrap()).unwrap())
        .filter(|message| message["method"] == "turn/start")
        .map(|message| message["params"].clone())
        .collect::<Vec<_>>();
    assert_eq!(turn_starts.len(), 1);
    assert!(validator.is_valid(&turn_starts[0]), "{}", turn_starts[0]);
    assert_eq!(turn_starts[0]["input"][1]["type"], "localImage");
    assert_answer(output.final_assistant_text);
}

#[test]
#[ignore = "runs the installed claude CLI against a signed-in account"]
fn installed_claude_code_accepts_image_blocks_and_file_references() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let output = RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(request(temp.path(), "claude-code"))
        .expect("installed claude turn");
    // The image is not referenced in the text, so a colour proves the native
    // image block arrived, unless the agent went looking for the file.
    let events = serde_json::to_string(&output.events).expect("events");
    assert!(
        !events.contains("swatch.png"),
        "agent opened the image: {events}"
    );
    assert!(
        events.contains("word.txt"),
        "agent never read the file: {events}"
    );
    assert_answer(output.final_assistant_text);
}

const STEER_PROMPT: &str = "Run the shell command `sleep 20 && echo SLEPT` and then reply with its output. Do nothing else.";
const STEER_TEXT: &str = "Change of plan: start your final reply with the word PINEAPPLE.";

fn steer_request(dir: &std::path::Path, provider: &str, prompt: &str) -> AgentTurnRequest {
    let mut request = request(dir, provider);
    request.prompt = prompt.to_string();
    request.metadata.clear();
    request
}

/// Runs a steerable turn on a thread and returns once the agent's first tool
/// call has started, so whatever the caller does next lands mid-turn.
fn start_tool_turn(
    request: AgentTurnRequest,
    steering: SessionSteeringHandle,
) -> (
    Arc<Mutex<Vec<(Instant, TurnStreamEvent)>>>,
    std::thread::JoinHandle<
        Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError>,
    >,
) {
    let live = Arc::new(Mutex::new(Vec::new()));
    let sink_live = Arc::clone(&live);
    let run = std::thread::spawn(move || {
        RustLlmAgentTurnBackend::new(Client::new()).run_steerable_turn(
            request,
            steering,
            Some(Arc::new(move |event| {
                sink_live.lock().unwrap().push((Instant::now(), event))
            })),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    while !live
        .lock()
        .unwrap()
        .iter()
        .any(|(_, event)| event.kind == TurnStreamEventKind::ToolCallStarted)
    {
        assert!(Instant::now() < deadline, "no tool call started");
        assert!(!run.is_finished(), "turn ended before its tool call");
        std::thread::sleep(Duration::from_millis(100));
    }
    (live, run)
}

/// A message steered while the agent's command runs is delivered into the
/// same turn and shapes its final reply. Prints the observed timing: Codex
/// takes `turn/steer` at once, mid-command; Claude Code 2.1.296 reads the
/// stdin user message at its next model request, after the running command
/// finishes and before the final reply, still within the turn.
fn assert_steers_mid_turn(provider: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let steering = SessionSteeringHandle::new();
    let (live, run) = start_tool_turn(
        steer_request(temp.path(), provider, STEER_PROMPT),
        steering.clone(),
    );
    std::thread::sleep(Duration::from_secs(3));
    let queued = Instant::now();
    assert!(steering.queue_steering_with_metadata(
        STEER_TEXT,
        BTreeMap::from([("id".to_string(), json!("steer-live"))]),
    ));
    let output = run.join().expect("thread").expect("steered turn");
    let ended = Instant::now();

    let live = live.lock().unwrap();
    let injected = live
        .iter()
        .find(|(_, event)| event.kind == TurnStreamEventKind::Other("steering_injected".into()))
        .unwrap_or_else(|| panic!("{provider}: steering never delivered into the turn"));
    assert_eq!(injected.1.message.as_deref(), Some(STEER_TEXT));
    assert_eq!(
        injected.1.details.as_ref().unwrap()["id"],
        json!("steer-live")
    );
    let tool_done = live
        .iter()
        .find(|(_, event)| event.kind == TurnStreamEventKind::ToolCallCompleted)
        .map(|(at, _)| at.duration_since(queued));
    eprintln!(
        "{provider}: delivered {:?} after queueing (command finished {:?} after queueing), turn ended {:?} after queueing",
        injected.0.duration_since(queued),
        tool_done,
        ended.duration_since(queued),
    );
    let text = output.final_assistant_text.unwrap_or_default();
    eprintln!("{provider}: final reply {text:?}");
    assert!(
        text.contains("PINEAPPLE"),
        "{provider}: steering not applied: {text}"
    );
    assert!(
        steering.close_and_drain().is_empty(),
        "{provider}: left undelivered"
    );
}

/// Interrupt stops the running command and settles the turn promptly.
fn assert_interrupts_mid_turn(
    provider: &str,
) -> Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError> {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = format!("sleep 61{}", if provider == "codex" { 1 } else { 2 });
    let steering = SessionSteeringHandle::new();
    let (_live, run) = start_tool_turn(
        steer_request(
            temp.path(),
            provider,
            &format!("Run the shell command `{marker}` and then reply DONE."),
        ),
        steering.clone(),
    );
    std::thread::sleep(Duration::from_secs(2));
    let interrupted = Instant::now();
    assert!(steering.interrupt());
    let result = run.join().expect("thread");
    let settled = interrupted.elapsed();
    eprintln!(
        "{provider}: interrupt settled after {settled:?}: {:?}",
        result
            .as_ref()
            .map(|output| output.final_assistant_text.clone())
            .map_err(|error| error.message.clone())
    );
    assert!(
        settled < Duration::from_secs(15),
        "{provider}: interrupt took {settled:?}"
    );
    let still_running = Command::new("pgrep")
        .args(["-x", "-f", &marker])
        .status()
        .expect("pgrep")
        .success();
    assert!(!still_running, "{provider}: `{marker}` still running");
    result
}

#[test]
#[ignore = "runs the installed codex app-server against a signed-in account"]
fn installed_codex_steers_into_running_turn_and_interrupts() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    assert_steers_mid_turn("codex");
    let error = assert_interrupts_mid_turn("codex").expect_err("interrupted turn");
    assert_eq!(error.message, "Stopped.");
}

#[test]
#[ignore = "runs the installed claude CLI against a signed-in account"]
fn installed_claude_code_steers_into_running_turn_and_interrupts() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    assert_steers_mid_turn("claude-code");
    let output = assert_interrupts_mid_turn("claude-code").expect("stopped turn");
    assert_eq!(output.final_assistant_text.as_deref(), Some("Stopped."));
}
