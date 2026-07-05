use std::env;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

fn main() {
    let mode =
        env::var("SPARK_FAKE_CODEX_APP_SERVER_MODE").unwrap_or_else(|_| "default".to_string());
    let log_path = env::var_os("SPARK_FAKE_CODEX_APP_SERVER_LOG").map(PathBuf::from);
    if mode == "steerable" {
        run_steerable(log_path);
    } else {
        run_default(log_path, mode == "model-list", mode == "request-user-input");
    }
}

fn run_default(log_path: Option<PathBuf>, model_list_only: bool, request_user_input: bool) {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut awaiting_request_user_input_response = false;

    for raw in stdin.lock().lines().map_while(Result::ok) {
        let message = serde_json::from_str::<Value>(&raw).expect("json-rpc request");
        log_message(log_path.as_ref(), &message);
        if awaiting_request_user_input_response {
            awaiting_request_user_input_response = false;
            emit_turn_completion(
                &mut stdout,
                "thread-test",
                "turn-test",
                "msg-test",
                "Ack",
                true,
            );
            continue;
        }

        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => write_json(
                &mut stdout,
                json!({"id": request_id, "result": {"userAgent": "fake"}}),
            ),
            "initialized" => {}
            "thread/start" if !model_list_only => write_json(
                &mut stdout,
                json!({"id": request_id, "result": {"thread": {"id": "thread-test"}}}),
            ),
            "turn/start" if !model_list_only => {
                write_json(
                    &mut stdout,
                    json!({"id": request_id, "result": {"turn": {"id": "turn-test", "status": "inProgress", "items": []}}}),
                );
                if request_user_input {
                    write_json(
                        &mut stdout,
                        json!({
                            "id": "server-request-1",
                            "method": "item/tool/requestUserInput",
                            "params": {
                                "threadId": "thread-test",
                                "turnId": "turn-test",
                                "itemId": "input-test",
                                "questions": [{
                                    "id": "choice",
                                    "header": "Choice",
                                    "question": "Pick one",
                                    "options": [{"label": "A", "description": "Use A"}]
                                }],
                                "autoResolutionMs": null
                            }
                        }),
                    );
                    awaiting_request_user_input_response = true;
                } else {
                    emit_turn_completion(
                        &mut stdout,
                        "thread-test",
                        "turn-test",
                        "msg-test",
                        "Ack",
                        true,
                    );
                }
            }
            "model/list" => write_json(
                &mut stdout,
                json!({
                    "id": request_id,
                    "result": {"data": [{
                        "id": "gpt-codex-test",
                        "model": "gpt-codex-test",
                        "displayName": "Codex Test",
                        "description": "Test model",
                        "isDefault": true,
                        "hidden": false,
                        "defaultReasoningEffort": "medium",
                        "supportedReasoningEfforts": [{"reasoningEffort": "medium", "description": "Medium"}]
                    }]}
                }),
            ),
            _ => write_json(
                &mut stdout,
                json!({"id": request_id, "error": {"code": -32601, "message": format!("unexpected method {method}")}}),
            ),
        }
    }
}

fn run_steerable(log_path: Option<PathBuf>) {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for raw in stdin.lock().lines().map_while(Result::ok) {
        let message = serde_json::from_str::<Value>(&raw).expect("json-rpc request");
        log_message(log_path.as_ref(), &message);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => write_json(
                &mut stdout,
                json!({"id": request_id, "result": {"userAgent": "fake"}}),
            ),
            "initialized" => {}
            "thread/start" => write_json(
                &mut stdout,
                json!({"id": request_id, "result": {"thread": {"id": "thread-steer"}}}),
            ),
            "turn/start" => write_json(
                &mut stdout,
                json!({"id": request_id, "result": {"turn": {"id": "turn-steer", "status": "inProgress", "items": []}}}),
            ),
            "turn/steer" => {
                write_json(&mut stdout, json!({"id": request_id, "result": {}}));
                emit_turn_completion(
                    &mut stdout,
                    "thread-steer",
                    "turn-steer",
                    "msg-steer",
                    "Steered",
                    false,
                );
            }
            _ => write_json(
                &mut stdout,
                json!({"id": request_id, "error": {"code": -32601, "message": format!("unexpected method {method}")}}),
            ),
        }
    }
}

fn emit_turn_completion(
    stdout: &mut impl Write,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    text: &str,
    include_usage: bool,
) {
    write_json(
        stdout,
        json!({"method": "item/agentMessage/delta", "params": {"threadId": thread_id, "turnId": turn_id, "itemId": item_id, "delta": text}}),
    );
    write_json(
        stdout,
        json!({"method": "item/completed", "params": {"threadId": thread_id, "turnId": turn_id, "item": {"type": "AgentMessage", "id": item_id, "content": [{"type": "Text", "text": text}], "phase": "final_answer"}}}),
    );
    if include_usage {
        write_json(
            stdout,
            json!({"method": "thread/tokenUsage/updated", "params": {"threadId": thread_id, "turnId": turn_id, "tokenUsage": {"total": {"inputTokens": 2, "cachedInputTokens": 0, "outputTokens": 1, "totalTokens": 3}}}}),
        );
    }
    write_json(
        stdout,
        json!({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "completed"}}}),
    );
}

fn write_json(stdout: &mut impl Write, value: Value) {
    writeln!(stdout, "{value}").expect("write json-rpc response");
    stdout.flush().expect("flush json-rpc response");
}

fn log_message(log_path: Option<&PathBuf>, message: &Value) {
    let Some(log_path) = log_path else {
        return;
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open fake codex app-server log");
    writeln!(file, "{message}").expect("write fake codex app-server log");
}
