//! Fake Claude Code CLI for process contracts: speaks just enough of the
//! `claude -p --output-format stream-json` surface to exercise the backend.

use serde_json::{json, Value};
use std::env;
use std::io::{Read, Write};

fn main() {
    // Bound blocked protocol contracts so a missing answer fails instead of hanging the gate.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(15));
        eprintln!("fake Claude Code timed out waiting for control traffic");
        std::process::exit(3);
    });
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut model = String::from("claude-opus-4-8");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--model" {
            if let Some(value) = iter.next() {
                model = value.clone();
            }
        }
    }

    if let Ok(log_path) = env::var("SPARK_FAKE_CLAUDE_CODE_LOG") {
        if !log_path.trim().is_empty() {
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let background =
                    env::var("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS").unwrap_or_default();
                let _ = writeln!(
                    log,
                    "{}\nCLAUDE_CODE_DISABLE_BACKGROUND_TASKS={background}\n-- invocation --",
                    args.join("\n")
                );
            }
        }
    }

    // Control-protocol mode: with stream-json input the caller drives via
    // control requests (model discovery) instead of a prompt turn, and keeps
    // stdin open, so answer line by line instead of reading to EOF.
    if args
        .windows(2)
        .any(|pair| pair[0] == "--input-format" && pair[1] == "stream-json")
    {
        let stdin = std::io::stdin();
        let mut line = String::new();
        while stdin
            .read_line(&mut line)
            .map(|read| read > 0)
            .unwrap_or(false)
        {
            if let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) {
                if message["type"] == "user" {
                    assert_eq!(message["message"]["role"], "user");
                    assert!(message["message"]["content"].as_str().is_some());
                    break;
                }
                if message["type"] == "control_request"
                    && message["request"]["subtype"] == "list_models"
                {
                    println!(
                        "{}",
                        serde_json::json!({
                            "type": "control_response",
                            "response": {
                                "subtype": "success",
                                "request_id": message["request_id"],
                                "response": {"models": [
                                    {
                                        "value": "default",
                                        "displayName": "Default (recommended)",
                                        "resolvedModel": "claude-fable-5[1m]",
                                    },
                                    {
                                        "value": "claude-fable-5[1m]",
                                        "displayName": "Fable",
                                        "supportsEffort": true,
                                    },
                                    {"value": "sonnet", "displayName": "Sonnet"},
                                ]},
                            },
                        })
                    );
                    return;
                }
            }
            line.clear();
        }
    }

    let mode = env::var("SPARK_FAKE_CLAUDE_CODE_MODE").unwrap_or_default();
    if mode == "resume-failure" && args.iter().any(|arg| arg == "--resume") {
        eprintln!("simulated resumed-session failure");
        std::process::exit(1);
    }
    let session_id = if mode == "resume-failure" {
        "sess-fake-claude-fresh"
    } else {
        "sess-fake-claude-1"
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let emit = |out: &mut dyn Write, mut line: serde_json::Value| {
        if mode == "legacy" && line["type"] == "system" {
            line.as_object_mut().unwrap().remove("capabilities");
        }
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    };

    emit(
        &mut out,
        serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": session_id,
            "model": model,
            "tools": ["Bash", "Read", "Edit", "AskUserQuestion"],
            "capabilities": if mode == "legacy" { vec![] } else { vec!["interrupt_receipt_v1", "msg_lifecycle_v1"] },
        }),
    );

    if mode == "legacy" {
        let mut remaining = String::new();
        std::io::stdin().read_to_string(&mut remaining).unwrap();
        assert!(remaining.is_empty(), "legacy must receive EOF after prompt");
    }
    if mode.starts_with("malformed") {
        if mode == "malformed-json" {
            writeln!(out, "{{invalid").unwrap();
            out.flush().unwrap();
        } else if mode == "malformed-response" {
            emit(
                &mut out,
                json!({"type": "control_response", "response": {"subtype": "success", "request_id": "unknown", "response": {"still_queued": []}}}),
            );
        } else {
            emit(
                &mut out,
                json!({"type": "control_request", "request": {"subtype": "can_use_tool"}}),
            );
        }
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "ignored", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "true"}}}),
        );
        let mut remaining = String::new();
        std::io::stdin().read_to_string(&mut remaining).unwrap();
        assert!(remaining.is_empty(), "must cease control responses");
    }
    if mode == "question"
        || mode == "dead-question"
        || mode == "dead-question-exit"
        || mode == "interrupt-question"
    {
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-prompt-tool", "stdio"]));
        let input = json!({"questions": [{"question": "Which color?", "header": "Color", "options": [{"label": "Red", "description": "Warm"}, {"label": "Blue", "description": "Cool"}], "multiSelect": false}]});
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "permission-1", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "true"}, "tool_use_id": "tool-permission"}}),
        );
        assert_eq!(
            read_control(),
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "permission-1", "response": {"behavior": "allow", "updatedInput": {"command": "true"}}}})
        );
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {
                "role": "assistant", "content": [{"type": "tool_use", "id": "tool-question", "name": "AskUserQuestion", "input": input}]
            }}),
        );
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "question-1", "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": input, "tool_use_id": "tool-question", "display_name": "AskUserQuestion", "requires_user_interaction": true}}),
        );
        if mode == "dead-question-exit" {
            std::process::exit(1);
        }
        if mode == "question" {
            let mut answered = input.clone();
            answered["answers"] = json!({"Which color?": "Blue"});
            assert_eq!(
                read_control(),
                json!({"type": "control_response", "response": {"subtype": "success", "request_id": "question-1", "response": {"behavior": "allow", "updatedInput": answered}}})
            );
        }
    }
    if mode == "interrupt"
        || mode == "interrupt-question"
        || mode == "interrupt-wrong-id"
        || mode == "interrupt-aborted"
        || mode == "interrupt-error"
    {
        if mode != "interrupt-question" {
            emit(
                &mut out,
                json!({"type": "stream_event", "session_id": session_id,
            "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Shortened turn."}}}),
            );
        }
        let interrupt = read_control();
        assert_eq!(interrupt["type"], "control_request");
        assert_eq!(interrupt["request"], json!({"subtype": "interrupt"}));
        assert!(interrupt["request_id"].as_str().is_some());
        if mode == "interrupt-question" {
            emit(
                &mut out,
                json!({"type": "control_cancel_request", "request_id": "question-1"}),
            );
        }
        emit(
            &mut out,
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": if mode == "interrupt-wrong-id" { json!("wrong") } else { interrupt["request_id"].clone() }, "response": {"still_queued": []}}}),
        );
        if mode == "interrupt-question" {
            emit(
                &mut out,
                json!({"type": "user", "session_id": session_id, "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "tool-question", "is_error": true, "content": "User rejected tool use"}]}, "tool_result_meta": [{"id": "tool-question", "non_execution_kind": "user-rejected"}]}),
            );
            emit(
                &mut out,
                json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
                "terminal_reason": "aborted_tools", "session_id": session_id,
                "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"]}),
            );
            std::process::exit(1);
        }
    }

    if mode == "interrupt-aborted" || mode == "interrupt-error" {
        emit(
            &mut out,
            json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
            "terminal_reason": if mode == "interrupt-error" { "api_error" } else { "aborted_streaming" },
            "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"],
            "session_id": session_id, "num_turns": 2, "usage": {"input_tokens": 0, "output_tokens": 0}}),
        );
        std::process::exit(1);
    }

    if mode == "error" {
        emit(
            &mut out,
            serde_json::json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "result": "simulated failure",
                "session_id": session_id,
                "num_turns": 1,
                "usage": {"input_tokens": 3, "output_tokens": 1},
            }),
        );
        return;
    }

    if mode == "result-only" || mode == "empty-result" {
        emit(
            &mut out,
            serde_json::json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": if mode == "empty-result" { "" } else { "Result without blocks." },
                "session_id": session_id,
                "usage": {"input_tokens": 1, "output_tokens": 1},
            }),
        );
        return;
    }

    if mode == "tool-payloads" {
        for (id, name, input, output) in [
            (
                "toolu_bash_described",
                "Bash",
                serde_json::json!({"command": "cargo test", "description": "Run the tests"}),
                "tests passed",
            ),
            (
                "toolu_bash_plain",
                "Bash",
                serde_json::json!({"command": "printf first\nprintf second"}),
                "first\nsecond",
            ),
            (
                "toolu_read",
                "Read",
                serde_json::json!({"file_path": "/tmp/example.rs"}),
                "fn main() {}",
            ),
            (
                "toolu_unknown",
                "McpWidget",
                serde_json::json!({"value": 1}),
                "done",
            ),
        ] {
            emit(
                &mut out,
                serde_json::json!({"type": "assistant", "session_id": session_id,
                "message": {"role": "assistant", "content": [
                    {"type": "tool_use", "id": id, "name": name, "input": input}
                ]}}),
            );
            emit(
                &mut out,
                serde_json::json!({"type": "user", "session_id": session_id,
                "message": {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": id, "content": output}
                ]}}),
            );
        }
        emit(
            &mut out,
            serde_json::json!({"type": "result", "subtype": "success",
            "is_error": false, "result": "Tools complete.", "session_id": session_id,
            "usage": {"input_tokens": 1, "output_tokens": 1}}),
        );
        return;
    }

    if mode == "multi-block" {
        for line in [
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "message_start"}}),
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "First block."}}}),
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 1,
                    "delta": {"type": "text_delta", "text": "Second block."}}}),
            serde_json::json!({"type": "assistant", "session_id": session_id,
            "message": {"role": "assistant", "content": [
                {"type": "text", "text": "First block."},
                {"type": "text", "text": "Second block."}
            ]}}),
            serde_json::json!({"type": "result", "subtype": "success", "is_error": false,
                "result": "Second block.", "session_id": session_id,
                "usage": {"input_tokens": 1, "output_tokens": 2}}),
        ] {
            emit(&mut out, line);
        }
        return;
    }

    let partials = mode != "no-partials";
    if partials {
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "message_start"}}),
        );
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "thinking_delta", "thinking": "I should inspect before changing anything."}}}),
        );
    }

    emit(
        &mut out,
        serde_json::json!({
            "type": "assistant",
            "session_id": session_id,
            "message": {
                "role": "assistant",
                "content": [{"type": "thinking", "thinking": "I should inspect before changing anything."}],
            },
        }),
    );
    if partials {
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 1,
                    "delta": {"type": "text_delta", "text": "Inspecting the repository."}}}),
        );
    }
    emit(
        &mut out,
        serde_json::json!({"type": "assistant", "session_id": session_id,
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Inspecting the repository."}
        ]}}),
    );
    emit(
        &mut out,
        serde_json::json!({"type": "assistant", "session_id": session_id,
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}
        ]}}),
    );
    emit(
        &mut out,
        serde_json::json!({
            "type": "user",
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "README.md"},
                ],
            },
        }),
    );
    if partials {
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "message_start"}}),
        );
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "The repository looks ready; running tests."}}}),
        );
    }
    emit(
        &mut out,
        serde_json::json!({
            "type": "assistant",
            "session_id": session_id,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "The repository looks ready; running tests."}],
            },
        }),
    );
    emit(
        &mut out,
        serde_json::json!({"type": "assistant", "session_id": session_id,
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_2", "name": "Bash", "input": {"command": "cargo test"}}
        ]}}),
    );
    emit(
        &mut out,
        serde_json::json!({
            "type": "user",
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "ok"},
                ],
            },
        }),
    );
    if partials {
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "message_start"}}),
        );
        emit(
            &mut out,
            serde_json::json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "All set."}}}),
        );
    }
    emit(
        &mut out,
        serde_json::json!({
            "type": "assistant",
            "session_id": session_id,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "All set."}],
            },
        }),
    );
    emit(
        &mut out,
        serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": "All set.",
            "session_id": session_id,
            "num_turns": 2,
            "duration_ms": 1200,
            "total_cost_usd": 0.0,
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "cache_read_input_tokens": 5,
                "cache_creation_input_tokens": 2,
            },
        }),
    );
}

fn read_control() -> Value {
    let mut line = String::new();
    assert!(
        std::io::stdin().read_line(&mut line).unwrap() > 0,
        "expected control message before EOF"
    );
    serde_json::from_str(&line).unwrap()
}
