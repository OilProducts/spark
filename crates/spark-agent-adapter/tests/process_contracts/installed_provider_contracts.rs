//! Attachment input against the installed `codex` and `claude` binaries.
//! These spend real model turns and need signed-in providers, so they are
//! opt-in:
//! `cargo test --workspace --all-features --test process_contracts installed_ -- --ignored`

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use base64::Engine as _;
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentAttachment, AgentTurnBackend, AgentTurnRequest, CodexAppServerBackend,
    RustLlmAgentTurnBackend, AGENT_ATTACHMENTS_METADATA_KEY,
};
use spark_common::debug::{CODEX_JSONRPC_TRACE_PATH_METADATA_KEY, ENV_SPARK_DEBUG_CODEX_JSONRPC};
use unified_llm_adapter::Client;

use super::test_support::ENV_LOCK;

/// A 16x16 solid red PNG.
const RED_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVR4nGP4z8BAEiJN9aiGUQ1DSgMAkPn/Afnh+ngAAAAASUVORK5CYII=";
const PROMPT: &str = "What single colour fills the attached image, and what one word is in the attached text file? Reply only as colour,word in lowercase.";

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

fn assert_answer(answer: Option<String>) {
    let answer = answer.unwrap_or_default().to_lowercase();
    assert!(
        answer.contains("red") && answer.contains("pineapple"),
        "unexpected answer: {answer}"
    );
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
    assert_answer(output.final_assistant_text);

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
}

#[test]
#[ignore = "runs the installed claude CLI against a signed-in account"]
fn installed_claude_code_accepts_image_blocks_and_file_references() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let output = RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(request(temp.path(), "claude-code"))
        .expect("installed claude turn");
    assert_answer(output.final_assistant_text);
}
