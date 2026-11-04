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
    AgentAttachment, AgentTurnBackend, AgentTurnEventSink, AgentTurnRequest, ClaudeCodeBackend,
    CodexAppServerBackend, RustLlmAgentTurnBackend, AGENT_ATTACHMENTS_METADATA_KEY,
};
use spark_common::debug::{CODEX_JSONRPC_TRACE_PATH_METADATA_KEY, ENV_SPARK_DEBUG_CODEX_JSONRPC};
use spark_common::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind};
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
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// Thinking blocks and blocks with text in the installed CLI's stream.
fn claude_thinking_blocks(
    config: &std::path::Path,
    model: &str,
    prompt: &str,
    options: &[&str],
) -> (usize, usize) {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--tools",
            "",
            "--model",
            model,
        ])
        .args(options)
        .env("CLAUDE_CONFIG_DIR", config)
        .current_dir(cwd.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("claude");
    let message = json!({"type": "user", "message": {"role": "user", "content": prompt}});
    let mut stdin = child.stdin.take().expect("stdin");
    std::io::Write::write_all(&mut stdin, format!("{message}\n").as_bytes()).expect("prompt");
    drop(stdin);
    let output = child.wait_with_output().expect("claude output");
    let mut blocks = (0, 0);
    let mut succeeded = false;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        succeeded |= message["type"] == "result" && message["is_error"] == false;
        if message["type"] != "assistant" {
            continue;
        }
        for block in message["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if block["type"] == "thinking" {
                blocks.0 += 1;
                blocks.1 +=
                    usize::from(!block["thinking"].as_str().unwrap_or("").trim().is_empty());
            }
        }
    }
    assert!(
        succeeded,
        "{model} {options:?} failed: {:?}",
        output.stdout.len()
    );
    blocks
}

/// Before/after evidence for `--thinking-display`, a hidden flag: this also
/// notices if a CLI update drops it or stops returning thinking text. Every run
/// gets a temporary CLAUDE_CONFIG_DIR, which has no sign-in of its own:
/// `CLAUDE_CODE_OAUTH_TOKEN=... cargo test --workspace --all-features --test
/// process_contracts installed_claude_code_thinking_display -- --ignored --nocapture`
/// Without a token, SPARK_THINKING_PROBE_CONFIG_DIR borrows a signed-in config
/// dir instead: its settings are not loaded and no session is saved in it.
#[test]
#[ignore = "runs the installed claude CLI; needs CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY or SPARK_THINKING_PROBE_CONFIG_DIR"]
fn installed_claude_code_thinking_display_returns_text_the_default_withholds() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let signed_in =
        std::env::var_os("SPARK_THINKING_PROBE_CONFIG_DIR").map(std::path::PathBuf::from);
    let borrowed: &[&str] = if signed_in.is_some() {
        &[
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--no-session-persistence",
        ]
    } else {
        &[]
    };
    assert!(
        signed_in.is_some()
            || ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"]
                .iter()
                .any(|key| std::env::var_os(key).is_some()),
        "a temporary config has no sign-in: set CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY or SPARK_THINKING_PROBE_CONFIG_DIR"
    );
    // A parent Claude Code session's own markers must not reach these runs.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if (name.starts_with("CLAUDE_CODE_") && name != "CLAUDE_CODE_OAUTH_TOKEN")
            || name == "CLAUDECODE"
        {
            std::env::remove_var(&key);
        }
    }
    let version = Command::new("claude")
        .arg("--version")
        .output()
        .expect("version");
    eprintln!("CLI: {}", String::from_utf8_lossy(&version.stdout).trim());
    let prompts = [
        "Which is larger, 2^100 or 3^63? Reason carefully, then answer in one word.",
        "Find the smallest positive integer that leaves remainder 2 mod 3, 3 mod 5 and 2 mod 7. Think carefully, then output only the number.",
    ];
    let mut totals = [(0, 0); 2];
    for model in ["claude-opus-5-5", "claude-haiku-5-5"] {
        let modes: [&[&str]; 2] = [&[], &["--thinking-display", "summarized"]];
        for (index, options) in modes.into_iter().enumerate() {
            let mut blocks = (0, 0);
            for prompt in prompts {
                let config = tempfile::tempdir().expect("config");
                let run = claude_thinking_blocks(
                    signed_in.as_deref().unwrap_or(config.path()),
                    model,
                    prompt,
                    &[borrowed, options].concat(),
                );
                blocks = (blocks.0 + run.0, blocks.1 + run.1);
            }
            eprintln!(
                "{model} options={options:?}: {} blocks, {} with text",
                blocks.0, blocks.1
            );
            totals[index] = (totals[index].0 + blocks.0, totals[index].1 + blocks.1);
        }
    }
    eprintln!(
        "default: {:?}; summarized: {:?} (blocks, with text)",
        totals[0], totals[1]
    );
    assert_eq!(
        totals[0].1, 0,
        "the default now returns text: drop the flag?"
    );
    assert!(totals[1].0 > 0 && totals[1].1 == totals[1].0, "{totals:?}");

    // And through Spark's own launch path.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(temp.path().join("config")).expect("config");
    std::env::set_var(
        "SPARK_CLAUDE_CODE_CONFIG_DIR",
        signed_in
            .clone()
            .unwrap_or_else(|| temp.path().join("config")),
    );
    if signed_in.is_some() {
        let wrapper = temp.path().join("claude-probe");
        let script = format!(
            "#!/bin/sh\nexec claude {} \"$@\"\n",
            "--setting-sources '' --strict-mcp-config --no-session-persistence"
        );
        std::fs::write(&wrapper, script).expect("wrapper");
        std::fs::set_permissions(
            &wrapper,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("wrapper");
        std::env::set_var("SPARK_CLAUDE_CODE_BIN", wrapper);
    }
    let mut request = request(temp.path(), "claude-code");
    request.prompt = prompts[0].to_string();
    request.metadata.clear();
    request.model = Some("claude-opus-5-5".to_string());
    let live = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_events = live.clone();
    let sink: AgentTurnEventSink = std::sync::Arc::new(move |event: TurnStreamEvent| {
        if let Some(state) = event.thinking_state() {
            sink_events.lock().unwrap().push(state);
        }
    });
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(request, Some(sink))
        .expect("claude turn");
    std::env::remove_var("SPARK_CLAUDE_CODE_CONFIG_DIR");
    std::env::remove_var("SPARK_CLAUDE_CODE_BIN");
    let thoughts = output
        .events
        .iter()
        .filter(|event| {
            event.kind == TurnStreamEventKind::ContentCompleted
                && event.channel == Some(TurnStreamChannel::Reasoning)
        })
        .count();
    let live = live.lock().unwrap();
    eprintln!("Spark launch: {thoughts} thoughts with text; live updates: {live:?}");
    assert!(thoughts > 0, "no thinking text: {:?}", output.events);
    assert_eq!(live.first().map(|state| state.0), Some(true));
    assert!(live
        .last()
        .is_some_and(|(active, tokens)| !active && tokens.is_some()));
}
