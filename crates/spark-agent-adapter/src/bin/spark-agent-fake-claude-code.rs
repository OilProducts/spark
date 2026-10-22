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
    if args.first().map(String::as_str) == Some("auth") {
        fake_auth(&args[1..]);
    }
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
                // The file is gone once the turn ends, so record its content.
                let instructions = args
                    .windows(2)
                    .find(|pair| pair[0] == "--append-system-prompt-file")
                    .and_then(|pair| std::fs::read_to_string(&pair[1]).ok())
                    .map(|text| format!("\nAPPEND_SYSTEM_PROMPT={text}"))
                    .unwrap_or_default();
                let path_first = env::var_os("PATH")
                    .and_then(|path| env::split_paths(&path).next())
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default();
                let config_dir = env::var("CLAUDE_CONFIG_DIR").unwrap_or_default();
                let _ = writeln!(
                    log,
                    "{}\nCLAUDE_CODE_DISABLE_BACKGROUND_TASKS={background}\nBASH_MAX_TIMEOUT_MS={}\nCLAUDE_CONFIG_DIR={config_dir}\nPATH_FIRST={path_first}{instructions}\n-- invocation --",
                    args.join("\n"),
                    env::var("BASH_MAX_TIMEOUT_MS").unwrap_or_default()
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
                    assert!(
                        message["uuid"].as_str().is_some(),
                        "user messages carry a uuid"
                    );
                    assert_user_content(&message["message"]["content"]);
                    append_log(&format!("USER_CONTENT={}", message["message"]["content"]));
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
                                        "supportedEffortLevels": ["low", "high"],
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

    if mode == "question-before-init" {
        // Never sends system/init: the init fallback must answer the
        // outstanding question before it closes stdin.
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "early-q", "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": [{"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}]}}}),
        );
        let response = read_control();
        assert_eq!(response["response"]["request_id"], "early-q");
        assert_eq!(response["response"]["response"]["behavior"], "deny");
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "All set.", "session_id": session_id}),
        );
        return;
    }
    if mode == "request-before-init" {
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "early-1", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "true"}}}),
        );
        assert_eq!(
            read_control(),
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "early-1", "response": {"behavior": "allow", "updatedInput": {"command": "true"}}}})
        );
    }
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
            // An unknown id, then missing, null and non-string ids, which must
            // not be taken for the absent context-usage request.
            for response in [
                json!({"subtype": "success", "request_id": "unknown", "response": {"still_queued": []}}),
                json!({"subtype": "success", "response": {"totalTokens": 1, "maxTokens": 2}}),
                json!({"subtype": "success", "request_id": null, "response": {}}),
                json!({"subtype": "success", "request_id": 7, "response": {}}),
            ] {
                emit(
                    &mut out,
                    json!({"type": "control_response", "response": response}),
                );
            }
        } else {
            emit(
                &mut out,
                json!({"type": "control_request", "request": {"subtype": "can_use_tool"}}),
            );
        }
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "after-malformed", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "true"}}}),
        );
        assert_eq!(
            read_control(),
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "after-malformed", "response": {"behavior": "allow", "updatedInput": {"command": "true"}}}})
        );
    }
    if mode == "unknown-subtype" {
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "unknown-1", "request": {"subtype": "mystery"}}),
        );
        assert_eq!(
            read_control(),
            json!({"type": "control_response", "response": {"subtype": "error", "request_id": "unknown-1", "error": "unsupported control request subtype: mystery"}})
        );
    }
    if mode == "duplicate-id" {
        // Each repeat of an answered id still gets exactly one answer.
        for (id, request, error) in [
            (
                "dup-1",
                json!({"subtype": "can_use_tool"}),
                "can_use_tool needs a tool_name and an object input",
            ),
            (
                "dup-2",
                json!({"subtype": "mystery"}),
                "unsupported control request subtype: mystery",
            ),
        ] {
            emit(
                &mut out,
                json!({"type": "control_request", "request_id": id, "request": request}),
            );
            assert_eq!(
                read_control()["response"],
                json!({"subtype": "error", "request_id": id, "error": error})
            );
            emit(
                &mut out,
                json!({"type": "control_request", "request_id": id, "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "true"}}}),
            );
            assert_eq!(
                read_control()["response"],
                json!({"subtype": "error", "request_id": id, "error": "duplicate control request id"})
            );
        }
    }
    if mode == "question-denied" {
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "question-1", "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": [{"question": "Which color?", "options": [{"label": "Red"}]}]}}}),
        );
        let denial = read_control();
        assert_eq!(denial["response"]["request_id"], "question-1");
        assert_eq!(denial["response"]["response"]["behavior"], "deny");
        assert!(denial["response"]["response"]["message"]
            .as_str()
            .unwrap()
            .contains("best judgment"));
    }
    if mode == "exit-plan-mode" {
        // Probe 1: ExitPlanMode reaches can_use_tool with the plan as input.
        emit(
            &mut out,
            json!({"type": "control_request", "request_id": "exit-plan", "session_id": session_id, "request": {"subtype": "can_use_tool", "tool_name": "ExitPlanMode", "input": {"plan": "# Plan\n\n- Do the thing.", "planFilePath": "/tmp/plan.md"}}}),
        );
        let answer = read_control()["response"].clone();
        assert_eq!(answer["request_id"], "exit-plan");
        if args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"])
        {
            assert_eq!(answer["response"]["behavior"], "deny");
            assert!(answer["response"]["message"]
                .as_str()
                .unwrap()
                .contains("submitted to the user for review"));
        } else {
            assert_eq!(answer["response"]["behavior"], "allow");
        }
    }
    if mode == "background" || mode == "background-interrupt" {
        // Probe 3: a background task keeps the session working past the
        // first result, and its completion starts a follow-up turn.
        let usage = |input: u64, output: u64| {
            json!({"claude-opus-4-8": {"inputTokens": input, "outputTokens": output,
            "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0, "thinkingTokens": 0, "costUSD": 0.0}})
        };
        let tasks = |live: bool| {
            let mut tasks = vec![
                json!({"task_id": "watch", "task_type": "monitor", "description": "watcher", "ambient": true}),
            ];
            if live {
                tasks.push(
                    json!({"task_id": "build", "task_type": "local_bash", "description": "build"}),
                );
            }
            json!({"type": "system", "subtype": "background_tasks_changed", "session_id": session_id, "tasks": tasks})
        };
        emit(&mut out, tasks(true));
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Started a background build."}]}}),
        );
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Started a background build.",
                "session_id": session_id, "result_index": 0, "queued_turn_count": 0, "modelUsage": usage(3, 30)}),
        );
        if mode == "background" {
            emit(
                &mut out,
                json!({"type": "system", "subtype": "task_notification", "session_id": session_id, "task_id": "build", "status": "completed"}),
            );
            emit(&mut out, tasks(false));
            emit(
                &mut out,
                json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Build finished."}]}}),
            );
            emit(
                &mut out,
                json!({"type": "result", "subtype": "success", "is_error": false, "result": "Build finished.",
                    "session_id": session_id, "result_index": 1, "queued_turn_count": 0, "modelUsage": usage(5, 50)}),
            );
        }
        drain_stdin();
        append_log("STDIN_CLOSED");
        return;
    }
    if mode == "mid-turn-continue" {
        // The text block streaming when the message arrives keeps streaming
        // the moment the CLI reads it.
        emit(
            &mut out,
            json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Working"}}}),
        );
        let message = read_control();
        let uuid = message["uuid"].as_str().expect("uuid").to_string();
        emit(
            &mut out,
            json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": " still."}}}),
        );
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Working still."}]}}),
        );
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Read your note."}]}}),
        );
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Read your note.",
                "session_id": session_id, "queued_turn_count": 0, "user_message_uuids": [uuid]}),
        );
        drain_stdin();
        return;
    }
    if mode == "mid-turn-message" || mode == "mid-turn-stop" {
        // Probe 4: a message with no priority folds into the running turn and
        // a later result lists its uuid.
        emit(
            &mut out,
            json!({"type": "stream_event", "session_id": session_id,
                "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Working."}}}),
        );
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Working."}]}}),
        );
        let message = read_control();
        assert_eq!(message["type"], "user");
        assert!(message.get("priority").is_none(), "no priority: {message}");
        let uuid = message["uuid"].as_str().expect("uuid").to_string();
        append_log(&format!("MID_TURN={}", message["message"]["content"]));
        if mode == "mid-turn-stop" {
            let interrupt = read_control();
            assert_eq!(interrupt["request"], json!({"subtype": "interrupt"}));
            // 2.1.296 names the message it still holds for a follow-up turn.
            emit(
                &mut out,
                json!({"type": "control_response", "response": {"subtype": "success", "request_id": interrupt["request_id"], "response": {"still_queued": [uuid]}}}),
            );
            emit(
                &mut out,
                json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
                    "terminal_reason": "aborted_streaming", "session_id": session_id, "queued_turn_count": 0}),
            );
        } else {
            // A result that does not list the message must not end the turn.
            emit(
                &mut out,
                json!({"type": "result", "subtype": "success", "is_error": false, "result": "Working.",
                    "session_id": session_id, "queued_turn_count": 0, "user_message_uuids": []}),
            );
            emit(
                &mut out,
                json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Read your note."}]}}),
            );
            emit(
                &mut out,
                json!({"type": "result", "subtype": "success", "is_error": false, "result": "Read your note.",
                    "session_id": session_id, "queued_turn_count": 0, "user_message_uuids": [uuid]}),
            );
        }
        drain_stdin();
        append_log("STDIN_CLOSED");
        return;
    }
    if mode == "queued-interrupt" {
        // A queued message holds the turn past the first result, so Stop must
        // interrupt the queued model turn instead of closing stdin.
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "First answer.",
                "session_id": session_id, "queued_turn_count": 1, "modelUsage": {"claude-opus-4-8": {"inputTokens": 1, "outputTokens": 1}}}),
        );
        let interrupt = read_control();
        assert_eq!(interrupt["request"], json!({"subtype": "interrupt"}));
        emit(
            &mut out,
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": interrupt["request_id"], "response": {"still_queued": []}}}),
        );
        emit(
            &mut out,
            json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
                "terminal_reason": "aborted_streaming", "session_id": session_id, "queued_turn_count": 0}),
        );
        drain_stdin();
        return;
    }
    if mode == "crash-after-init" || mode == "crash-orphan" || mode == "crash-chatty-orphan" {
        if !args.iter().any(|arg| arg == "--resume") {
            emit(
                &mut out,
                json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Working."}]}}),
            );
            if mode == "crash-orphan" {
                // A surviving descendant inherits stdout and stderr, so
                // neither reaches EOF when this process exits.
                std::process::Command::new("sleep")
                    .arg("30")
                    .spawn()
                    .unwrap();
            }
            if mode == "crash-chatty-orphan" {
                // The descendant keeps writing blank and non-JSON lines, so
                // stdout is never quiet after this process exits.
                std::process::Command::new("sh")
                    .args([
                        "-c",
                        "for i in $(seq 1500); do echo; echo noise; sleep 0.02; done",
                    ])
                    .spawn()
                    .unwrap();
            }
            std::process::exit(1);
        }
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Recovered.", "session_id": session_id}),
        );
        return;
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

    if mode == "error" || mode == "auth-error" || mode == "not-logged-in" {
        emit(
            &mut out,
            serde_json::json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "result": match mode.as_str() {
                    "auth-error" => {
                        "Failed to authenticate: OAuth session expired and could not be refreshed"
                    }
                    // What a never-signed-in CLAUDE_CONFIG_DIR ends a turn with.
                    "not-logged-in" => "Not logged in \u{b7} Please run /login",
                    _ => "simulated failure",
                },
                "session_id": session_id,
                "num_turns": 1,
                "usage": {"input_tokens": 3, "output_tokens": 1},
            }),
        );
        return;
    }

    if mode.starts_with("structured") {
        // `--json-schema`: the answer arrives as a StructuredOutput call and
        // the result's text is that object. The first attempt of the
        // violation and retries modes fails; a resumed repair answers validly.
        // The retries-*-text modes leave a parseable envelope in the result
        // or in assistant text; -always never stops exhausting retries.
        let resumed = args.iter().any(|arg| arg == "--resume");
        let usage = |tokens: u64| {
            json!({"claude-opus-4-8": {"inputTokens": tokens, "outputTokens": tokens,
            "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0, "thinkingTokens": 0, "costUSD": 0.0}})
        };
        let model_usage = usage(if resumed { 15 } else { 10 });
        if mode.starts_with("structured-retries") && (!resumed || mode.ends_with("-always")) {
            let leftover = json!({"outcome": "success", "preferred_label": "Rework"}).to_string();
            let mut result = json!({"type": "result", "subtype": "error_max_structured_output_retries",
                "is_error": true, "session_id": session_id, "modelUsage": model_usage});
            if mode.contains("result-text") {
                result["result"] = json!(leftover);
            }
            if mode.contains("assistant-text") {
                emit(
                    &mut out,
                    json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant",
                        "content": [{"type": "text", "text": leftover}]}}),
                );
            }
            emit(&mut out, result);
            return;
        }
        let answer = if mode == "structured-violation" && !resumed {
            json!({"outcome": "success", "context_updates": {"context.undeclared": 1}})
        } else {
            json!({"outcome": "success", "preferred_label": "Approve"})
        };
        emit(
            &mut out,
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_structured", "name": "StructuredOutput", "input": answer}
            ]}}),
        );
        emit(
            &mut out,
            json!({"type": "result", "subtype": "success", "is_error": false, "result": answer.to_string(),
                "structured_output": answer, "session_id": session_id, "modelUsage": model_usage}),
        );
        return;
    }

    if mode == "status-events" {
        // Frames as CLI 2.1.296 reports compaction, retries and rate limits.
        for frame in [
            json!({"type": "system", "subtype": "api_retry", "session_id": session_id, "attempt": 2, "max_retries": 10,
                "retry_delay_ms": 3500, "error_status": 529, "error": "overloaded"}),
            json!({"type": "rate_limit_event", "session_id": session_id, "rate_limit_info": {"status": "allowed",
                "resetsAt": 1_790_000_000, "rateLimitType": "five_hour"}}),
            json!({"type": "rate_limit_event", "session_id": session_id, "rate_limit_info": {"status": "allowed_warning",
                "resetsAt": 1_790_002_800, "rateLimitType": "five_hour",
                "unifiedWindows": {"five_hour": {"utilization": 0.82, "resetsAt": 1_790_002_800}}}}),
            json!({"type": "system", "subtype": "status", "session_id": session_id, "status": "compacting"}),
            json!({"type": "system", "subtype": "status", "session_id": session_id, "status": null, "compact_result": "success"}),
            json!({"type": "system", "subtype": "compact_boundary", "session_id": session_id,
                "compact_metadata": {"trigger": "auto", "pre_tokens": 21559, "post_tokens": 1887, "duration_ms": 4697}}),
            json!({"type": "user", "session_id": session_id, "message": {"role": "user",
                "content": "This session is being continued from a previous conversation. Summary: ..."}}),
            json!({"type": "assistant", "session_id": session_id, "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Done.", "session_id": session_id,
                "modelUsage": {"claude-opus-4-8": {"inputTokens": 3, "outputTokens": 30, "contextWindow": 1_000_000}}}),
        ] {
            emit(&mut out, frame);
        }
        drain_stdin();
        return;
    }
    if mode == "subagent" {
        // Frames as CLI 2.1.296 reports a synchronous Agent subagent.
        let parent = "toolu_agent";
        let sub = |content: serde_json::Value, role: &str| {
            json!({"type": role, "session_id": session_id, "parent_tool_use_id": parent,
                "agent_id": "agent-1", "message": {"role": role, "content": content}})
        };
        for frame in [
            json!({"type": "assistant", "session_id": session_id, "parent_tool_use_id": null, "message": {"role": "assistant", "content": [
                {"type": "text", "text": "Delegating."},
                {"type": "tool_use", "id": parent, "name": "Agent", "input": {
                    "description": "Find the config loader", "prompt": "Locate where config is loaded.", "subagent_type": "Explore"}}]}}),
            json!({"type": "system", "subtype": "task_started", "session_id": session_id, "task_id": "task-1", "tool_use_id": parent,
                "description": "Find the config loader", "subagent_type": "Explore", "task_type": "local_agent", "spawn_depth": 1,
                "prompt": "Locate where config is loaded."}),
            sub(
                json!([{"type": "text", "text": "Locate where config is loaded."}]),
                "user",
            ),
            json!({"type": "stream_event", "session_id": session_id, "parent_tool_use_id": parent,
                "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Subagent partial"}}}),
            sub(
                json!([{"type": "thinking", "thinking": "Subagent thinking"},
                {"type": "text", "text": "Subagent narration"},
                {"type": "tool_use", "id": "toolu_sub_grep", "name": "Grep", "input": {"pattern": "load_config", "path": "/src"}}]),
                "assistant",
            ),
            json!({"type": "system", "subtype": "task_progress", "session_id": session_id, "task_id": "task-1", "tool_use_id": parent,
                "description": "Running Grep", "usage": {"total_tokens": 1200, "tool_uses": 1, "duration_ms": 900}, "last_tool_name": "Grep"}),
            sub(
                json!([{"type": "tool_result", "tool_use_id": "toolu_sub_grep", "content": "src/config.rs"}]),
                "user",
            ),
            sub(
                json!([{"type": "text", "text": "Config loads in src/config.rs."}]),
                "assistant",
            ),
            json!({"type": "system", "subtype": "task_updated", "session_id": session_id, "task_id": "task-1",
                "patch": {"status": "completed", "end_time": 1_790_000_000}}),
            json!({"type": "system", "subtype": "task_notification", "session_id": session_id, "task_id": "task-1", "tool_use_id": parent,
                "status": "completed", "summary": "Found the config loader", "usage": {"total_tokens": 1500, "tool_uses": 1, "duration_ms": 1500}}),
            json!({"type": "user", "session_id": session_id, "parent_tool_use_id": null, "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": parent, "content": [{"type": "text", "text": "Config loads in src/config.rs."}]}]}}),
            json!({"type": "assistant", "session_id": session_id, "parent_tool_use_id": null, "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Done.", "session_id": session_id}),
        ] {
            emit(&mut out, frame);
        }
        drain_stdin();
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
            // Cumulative per session, as 2.1.28x reports it: a resumed turn
            // carries the first turn's usage plus its own.
            "modelUsage": if args.iter().any(|arg| arg == "--resume") {
                json!({
                    "claude-opus-4-8": {"inputTokens": 4, "outputTokens": 40,
                        "cacheReadInputTokens": 105, "cacheCreationInputTokens": 2,
                        "thinkingTokens": 0, "costUSD": 0.75},
                    "claude-haiku-4-5": {"inputTokens": 2, "outputTokens": 4,
                        "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0,
                        "thinkingTokens": 0, "costUSD": 0.01},
                })
            } else {
                json!({
                    "claude-opus-4-8": {"inputTokens": 3, "outputTokens": 30,
                        "cacheReadInputTokens": 5, "cacheCreationInputTokens": 2,
                        "thinkingTokens": 0, "costUSD": 0.5},
                    "claude-haiku-4-5": {"inputTokens": 2, "outputTokens": 4,
                        "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0,
                        "thinkingTokens": 0, "costUSD": 0.01},
                })
            },
        }),
    );
}

/// Reads stdin to EOF, answering the `get_context_usage` request Spark
/// sends once a turn's final result is in.
fn drain_stdin() {
    let mut line = String::new();
    while std::io::stdin()
        .read_line(&mut line)
        .map(|read| read > 0)
        .unwrap_or(false)
    {
        let message = serde_json::from_str::<Value>(&line).unwrap_or_default();
        if message["request"]["subtype"] == "get_context_usage" {
            append_log("GET_CONTEXT_USAGE");
            println!(
                "{}",
                json!({"type": "control_response", "response": {"subtype": "success", "request_id": message["request_id"],
                    "response": {"totalTokens": 24000, "maxTokens": 1000000, "percentage": 2.4, "categories": []}}})
            );
        }
        line.clear();
    }
}

fn read_control() -> Value {
    let mut line = String::new();
    assert!(
        std::io::stdin().read_line(&mut line).unwrap() > 0,
        "expected control message before EOF"
    );
    serde_json::from_str(&line).unwrap()
}

/// `claude auth status|login` as 2.1.28x prints them. The sign-in lives in a
/// marker file under CLAUDE_CONFIG_DIR; the code `good#state` completes login.
fn fake_auth(args: &[String]) -> ! {
    let marker = std::path::PathBuf::from(env::var("CLAUDE_CONFIG_DIR").unwrap_or_default())
        .join("fake-signed-in");
    if args.first().map(String::as_str) == Some("status") {
        let signed_in = marker.exists();
        let mut report =
            json!({"loggedIn": signed_in, "authMethod": "none", "apiProvider": "firstParty"});
        if signed_in {
            report["authMethod"] = json!("claude.ai");
            report["email"] = json!("spark@example.test");
            report["subscriptionType"] = json!("max");
        }
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        std::process::exit(if signed_in { 0 } else { 1 });
    }
    println!("Opening browser to sign in\u{2026}");
    println!("If the browser didn't open, visit: \u{1b}]8;;https://claude.com/cai/oauth/authorize?code=true&state=fake\u{7}https://claude.com/cai/oauth/authorize?code=true&state=fake\u{1b}]8;;\u{7}");
    print!("Paste code here if prompted > ");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    while std::io::stdin()
        .read_line(&mut line)
        .map(|read| read > 0)
        .unwrap_or(false)
    {
        match line.trim() {
            "good#state" => {
                std::fs::write(&marker, "").unwrap();
                println!("Login successful.");
                std::process::exit(0);
            }
            code if !code.contains('#') => {
                eprintln!("Invalid code. Please make sure the full code was copied.")
            }
            _ => {
                eprintln!("Login failed: Invalid authorization code");
                std::process::exit(1);
            }
        }
        line.clear();
    }
    std::process::exit(1)
}

/// The stream-json user content shapes the installed CLI accepts: a plain
/// string, or non-empty text blocks plus base64 image blocks in the formats
/// Claude reads natively.
fn assert_user_content(content: &Value) {
    if content.is_string() {
        return;
    }
    let blocks = content
        .as_array()
        .expect("content is a string or block array");
    assert!(!blocks.is_empty(), "content blocks must not be empty");
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => assert!(
                block["text"].as_str().is_some_and(|text| !text.is_empty()),
                "text blocks need text: {block}"
            ),
            Some("image") => {
                assert_eq!(block["source"]["type"], "base64", "{block}");
                assert!(
                    ["image/png", "image/jpeg", "image/gif", "image/webp"]
                        .contains(&block["source"]["media_type"].as_str().unwrap_or_default()),
                    "unsupported image media type: {block}"
                );
                assert!(
                    block["source"]["data"]
                        .as_str()
                        .is_some_and(|data| !data.is_empty()),
                    "image blocks need data: {block}"
                );
            }
            _ => panic!("unexpected content block: {block}"),
        }
    }
}

fn append_log(line: &str) {
    let Ok(log_path) = env::var("SPARK_FAKE_CLAUDE_CODE_LOG") else {
        return;
    };
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(log, "{line}");
    }
}
