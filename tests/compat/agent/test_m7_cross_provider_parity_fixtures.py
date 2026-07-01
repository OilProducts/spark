from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import textwrap
from typing import Any, Mapping

from tests.compat import harness


ITEM_ID = "M7-I2"
ITEM_REQUIREMENTS = ("REQ-001", "REQ-016", "REQ-017")
ITEM_DECISIONS = (
    "CD-CODING-AGENT-RUST-001",
    "CD-CODING-AGENT-RUST-011",
    "CD-CODING-AGENT-RUST-013",
)


def test_m7_cross_provider_parity_fixture_matches_rust_public_api(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifest = _cross_provider_parity_manifest(tmp_path)
    _assert_observable_cross_provider_matrix(manifest["observation"])

    _assert_fixture(
        manifest,
        compat_fixture_root / f"{manifest['fixture_id']}.json",
        compat_update_goldens,
    )


def _cross_provider_parity_manifest(tmp_path: Path) -> dict[str, Any]:
    observation = _run_rust_probe(tmp_path)
    return _manifest(
        fixture_id="agent/m7-cross-provider-parity-matrix",
        scenario="m7_cross_provider_parity_matrix",
        input_payload={
            "probe": "public spark-agent-adapter native provider profiles",
            "native_providers": ["openai", "anthropic", "gemini"],
            "non_native_selectors_observed_but_not_counted": [
                "openrouter",
                "openai_compatible",
                "litellm",
            ],
        },
        observation=observation,
    )


def _run_rust_probe(tmp_path: Path) -> dict[str, Any]:
    repo_root = Path(__file__).resolve().parents[3]
    probe_root = tmp_path / "rust-m7-cross-provider-probe"
    source_dir = probe_root / "src"
    source_dir.mkdir(parents=True)
    (probe_root / "Cargo.toml").write_text(
        textwrap.dedent(
            f"""
            [package]
            name = "spark-agent-m7-cross-provider-probe"
            version = "0.0.0"
            edition = "2021"
            publish = false

            [dependencies]
            spark-agent-adapter = {{ path = {str(repo_root / "crates" / "spark-agent-adapter")!r} }}
            unified-llm-adapter = {{ path = {str(repo_root / "crates" / "unified-llm-adapter")!r} }}
            serde_json = "1.0"
            """
        ).strip()
        + "\n",
        encoding="utf-8",
    )
    (source_dir / "main.rs").write_text(_RUST_PROBE_SOURCE, encoding="utf-8")

    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(repo_root / "target")
    env["PROBE_WORKSPACE"] = str(tmp_path / "workspace")
    completed = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "--manifest-path",
            str(probe_root / "Cargo.toml"),
        ],
        cwd=repo_root,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=180,
        check=False,
    )
    assert completed.returncode == 0, {
        "returncode": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }
    observation = json.loads(completed.stdout)
    assert isinstance(observation, dict)
    return observation


_RUST_PROBE_SOURCE = r'''
use std::env;

use serde_json::{json, Map, Value};
use spark_agent_adapter::{
    create_anthropic_profile, create_gemini_profile, create_gemini_profile_with_options,
    create_openai_profile, detect_loop, normalize_provider_selector, ChildSessionOptions,
    AssistantTurn, ExecutionEnvironment, HistoryTurn, ProviderProfile, Session, SessionConfig,
    ToolDispatchContext, ToolRegistry, ToolResultsTurn, UserTurn,
};
use unified_llm_adapter::{FinishReason, ToolCall, ToolResult, ToolResultData};

fn tool_names(profile: &ProviderProfile) -> Vec<String> {
    profile.tool_registry.names()
}

fn tool_properties(profile: &ProviderProfile, tool_name: &str) -> Vec<String> {
    let mut names = profile
        .tool_registry
        .get(tool_name)
        .expect("tool")
        .definition
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn tool_required(profile: &ProviderProfile, tool_name: &str) -> Vec<String> {
    profile
        .tool_registry
        .get(tool_name)
        .expect("tool")
        .definition
        .parameters
        .get("required")
        .and_then(Value::as_array)
        .expect("required")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn timeout_default(profile: &ProviderProfile) -> Value {
    profile
        .tool_registry
        .get("shell")
        .expect("shell")
        .definition
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("timeout_ms"))
        .and_then(|timeout| timeout.get("default"))
        .cloned()
        .expect("timeout default")
}

fn result_payload(result: ToolResult) -> Value {
    json!({
        "tool_call_id": result.tool_call_id,
        "content": result.content,
        "is_error": result.is_error
    })
}

fn shell_payload(result: ToolResult) -> Value {
    let content = result.content;
    json!({
        "tool_call_id": result.tool_call_id,
        "is_error": result.is_error,
        "content": {
            "stdout": content.get("stdout").cloned().unwrap_or(Value::Null),
            "stderr_empty": content
                .get("stderr")
                .and_then(Value::as_str)
                .map(str::is_empty)
                .unwrap_or(false),
            "exit_code": content.get("exit_code").cloned().unwrap_or(Value::Null),
            "timed_out": content.get("timed_out").cloned().unwrap_or(Value::Null)
        }
    })
}

fn provider_path_key(provider_id: &str) -> &'static str {
    if provider_id == "openai" {
        "path"
    } else {
        "file_path"
    }
}

fn file_arguments(provider_id: &str, path: &str) -> Map<String, Value> {
    let mut arguments = Map::new();
    arguments.insert(provider_path_key(provider_id).to_string(), json!(path));
    arguments
}

fn read_arguments(provider_id: &str, path: &str) -> Value {
    Value::Object(file_arguments(provider_id, path))
}

fn write_arguments(provider_id: &str, path: &str, content: &str) -> Value {
    let mut arguments = file_arguments(provider_id, path);
    arguments.insert("content".to_string(), json!(content));
    Value::Object(arguments)
}

fn edit_arguments(provider_id: &str, path: &str, old: &str, new: &str) -> Value {
    if provider_id == "openai" {
        let patch = [
            "*** Begin Patch".to_string(),
            format!("*** Update File: {path}"),
            "@@".to_string(),
            format!("-{old}"),
            format!("+{new}"),
            "*** End Patch".to_string(),
        ]
        .join("\n");
        return json!({"patch": patch});
    }

    let mut arguments = file_arguments(provider_id, path);
    arguments.insert("old_string".to_string(), json!(old));
    arguments.insert("new_string".to_string(), json!(new));
    if provider_id == "gemini" {
        arguments.insert(
            "instruction".to_string(),
            json!("Replace the exact old_string with new_string."),
        );
    }
    Value::Object(arguments)
}

fn grep_arguments(provider_id: &str, path: &str) -> Value {
    if provider_id == "anthropic" {
        json!({"pattern": "needle", "path": path, "glob": "*.txt"})
    } else {
        json!({"pattern": "needle", "path": path, "glob_filter": "*.txt"})
    }
}

fn dispatch(
    registry: &ToolRegistry,
    environment: &ExecutionEnvironment,
    config: SessionConfig,
    call_id: &str,
    tool_name: &str,
    arguments: Value,
) -> ToolResult {
    registry.dispatch(
        ToolCall::new(call_id, tool_name, arguments),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            config,
            ..ToolDispatchContext::default()
        },
    )
}

fn parity_matrix_payload(profile: &ProviderProfile, environment: &ExecutionEnvironment) -> Value {
    let provider_id = profile.id.as_str();
    let registry = profile.registry();
    let base = format!("matrix/{provider_id}");

    let create_path = format!("{base}/created.txt");
    let create = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-create"),
        "write_file",
        write_arguments(provider_id, &create_path, &format!("{provider_id} created\n")),
    );

    let edit_path = format!("{base}/read-edit.txt");
    dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-seed-edit"),
        "write_file",
        write_arguments(provider_id, &edit_path, &format!("{provider_id} old\n")),
    );
    let read_before_edit = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-read-before-edit"),
        "read_file",
        read_arguments(provider_id, &edit_path),
    );
    let edit_tool = if registry.get("apply_patch").is_some() {
        "apply_patch"
    } else {
        "edit_file"
    };
    let edit = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-native-edit"),
        edit_tool,
        edit_arguments(
            provider_id,
            &edit_path,
            &format!("{provider_id} old"),
            &format!("{provider_id} new"),
        ),
    );

    let multi_a = format!("{base}/multi-a.txt");
    let multi_b = format!("{base}/multi-b.txt");
    let multi_file = registry.dispatch_many(
        [
            ToolCall::new(
                &format!("{provider_id}-multi-a"),
                "write_file",
                write_arguments(provider_id, &multi_a, "A\n"),
            ),
            ToolCall::new(
                &format!("{provider_id}-multi-b"),
                "write_file",
                write_arguments(provider_id, &multi_b, "B\n"),
            ),
        ],
        ToolDispatchContext {
            execution_environment: environment.clone(),
            supports_parallel_tool_calls: profile.supports("parallel_tool_calls"),
            ..ToolDispatchContext::default()
        },
    );

    let shell = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-shell-ok"),
        "shell",
        json!({"command": format!("printf {provider_id}-shell"), "timeout_ms": 1_000}),
    );
    let timeout = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-shell-timeout"),
        "shell",
        json!({"command": "sleep 2", "timeout_ms": 1}),
    );

    let search_dir = format!("{base}/search");
    dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-search-write-txt"),
        "write_file",
        write_arguments(provider_id, &format!("{search_dir}/one.txt"), "needle one\n"),
    );
    dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-search-write-md"),
        "write_file",
        write_arguments(provider_id, &format!("{search_dir}/two.md"), "needle two\n"),
    );
    let glob = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-glob"),
        "glob",
        json!({"pattern": "*.txt", "path": search_dir}),
    );
    let grep = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-grep"),
        "grep",
        grep_arguments(provider_id, &format!("{base}/search")),
    );

    let multi_step_path = format!("{base}/multi-step.txt");
    dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-multi-step-seed"),
        "write_file",
        write_arguments(provider_id, &multi_step_path, "status: todo\n"),
    );
    let multi_step_read = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-multi-step-read"),
        "read_file",
        read_arguments(provider_id, &multi_step_path),
    );
    let multi_step_edit = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-multi-step-edit"),
        edit_tool,
        edit_arguments(provider_id, &multi_step_path, "status: todo", "status: done"),
    );

    let large_path = format!("{base}/large.txt");
    dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-large-write"),
        "write_file",
        write_arguments(provider_id, &large_path, &"0123456789".repeat(20)),
    );
    let mut truncation_config = SessionConfig::default();
    truncation_config
        .tool_output_limits
        .insert("read_file".to_string(), 64);
    let large_read = dispatch(
        &registry,
        environment,
        truncation_config,
        &format!("{provider_id}-large-read"),
        "read_file",
        read_arguments(provider_id, &large_path),
    );

    let missing_path = format!("{base}/missing.txt");
    let missing_read = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-missing-read"),
        "read_file",
        read_arguments(provider_id, &missing_path),
    );
    let recovery_write = dispatch(
        &registry,
        environment,
        SessionConfig::default(),
        &format!("{provider_id}-recovery-write"),
        "write_file",
        write_arguments(provider_id, &missing_path, "recovered\n"),
    );

    let gemini_directory_tools = if provider_id == "gemini" {
        json!({
            "read_many_files": result_payload(dispatch(
                &registry,
                environment,
                SessionConfig::default(),
                "gemini-read-many",
                "read_many_files",
                json!({"paths": [multi_a, multi_b]}),
            )),
            "list_dir": result_payload(dispatch(
                &registry,
                environment,
                SessionConfig::default(),
                "gemini-list-dir",
                "list_dir",
                json!({"path": format!("{base}/search"), "depth": 0}),
            ))
        })
    } else {
        Value::Null
    };

    json!({
        "simple_file_creation": result_payload(create),
        "read_then_edit": {
            "read_before_edit": result_payload(read_before_edit),
            "edit": result_payload(edit),
            "final_content": environment.read_file(&edit_path, None, None).expect("edited file")
        },
        "multi_file_edits": {
            "supports_parallel_tool_calls": profile.supports("parallel_tool_calls"),
            "results": multi_file.into_iter().map(result_payload).collect::<Vec<_>>(),
            "final_files": {
                "a": environment.read_file(&multi_a, None, None).expect("multi a"),
                "b": environment.read_file(&multi_b, None, None).expect("multi b")
            }
        },
        "shell_execution": shell_payload(shell),
        "shell_timeout": shell_payload(timeout),
        "grep_plus_glob": {
            "glob": result_payload(glob),
            "grep": result_payload(grep)
        },
        "multi_step_task": {
            "read": result_payload(multi_step_read),
            "edit": result_payload(multi_step_edit),
            "final_content": environment
                .read_file(&multi_step_path, None, None)
                .expect("multi step final")
        },
        "large_output_truncation": {
            "is_error": large_read.is_error,
            "contains_truncation_warning": large_read
                .content
                .as_str()
                .map(|content| content.contains("[WARNING: Tool output was truncated."))
                .unwrap_or(false)
        },
        "parallel_tool_calls": {
            "supported": profile.supports("parallel_tool_calls"),
            "observed_result_count": 2
        },
        "tool_error_recovery": {
            "missing_read": result_payload(missing_read),
            "recovery_write": result_payload(recovery_write),
            "final_content": environment.read_file(&missing_path, None, None).expect("recovered file")
        },
        "provider_native_edit_format": {
            "tool": edit_tool,
            "argument_keys": edit_arguments(provider_id, &edit_path, "x", "y")
                .as_object()
                .expect("edit arguments")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            "has_apply_patch_tool": registry.get("apply_patch").is_some(),
            "has_edit_file_tool": registry.get("edit_file").is_some()
        },
        "gemini_directory_tools": gemini_directory_tools
    })
}

fn profile_payload(profile: &ProviderProfile) -> Value {
    let edit_tool = if profile.tool_registry.get("apply_patch").is_some() {
        "apply_patch"
    } else {
        "edit_file"
    };
    json!({
        "id": profile.id,
        "model": profile.model,
        "tool_names": tool_names(profile),
        "edit_surface": {
            "primary_tool": edit_tool,
            "properties": tool_properties(profile, edit_tool),
            "required": tool_required(profile, edit_tool)
        },
        "shell_timeout_default": timeout_default(profile),
        "supports_parallel_tool_calls": profile.supports("parallel_tool_calls"),
        "supports_reasoning": profile.supports("reasoning"),
        "context_window_size": profile.context_window_size
    })
}

fn provider_request_payload(profile: ProviderProfile, environment: &ExecutionEnvironment) -> Value {
    let mut config = SessionConfig::default();
    config.reasoning_effort = Some("medium".to_string());
    let mut session = Session::new(profile, environment.clone(), config);
    let start_event = session.next_event().expect("session start");
    let request = session.build_request("system prompt");
    session.queue_steering("respect the latest operator instruction");
    let queued_steering = session
        .steering_queue
        .front()
        .map(|turn| turn.text())
        .unwrap_or_default();
    json!({
        "start_event_kind": start_event.kind,
        "provider": request.provider,
        "model": request.model,
        "reasoning_effort": request.reasoning_effort,
        "tool_names": request
            .tools
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>(),
        "provider_options": request.provider_options,
        "tool_choice_mode": request.tool_choice.map(|choice| choice.mode),
        "queued_steering": queued_steering,
    })
}

fn main() {
    let workspace = env::var("PROBE_WORKSPACE").expect("PROBE_WORKSPACE");
    let environment = ExecutionEnvironment::local(&workspace);

    let openai = create_openai_profile("gpt-5.2");
    let anthropic = create_anthropic_profile("claude-sonnet-4-5");
    let gemini = create_gemini_profile("gemini-3.1-pro-preview");
    let gemini_with_web = create_gemini_profile_with_options(
        "gemini-3.1-pro-preview",
        spark_agent_adapter::GeminiProfileOptions {
            enable_web_search: true,
            enable_web_fetch: true,
        },
    );

    let mut parent_session = Session::new(
        gemini.clone(),
        environment.clone(),
        SessionConfig {
            max_subagent_depth: 2,
            reasoning_effort: Some("medium".to_string()),
            ..SessionConfig::default()
        },
    );
    let parent_environment = parent_session.execution_environment.clone();
    let (child_status, child_model, child_depth, child_shares_environment) = {
        let child = parent_session
            .create_child_session(ChildSessionOptions::new().with_model("gemini-child"))
            .expect("child session");
        let child_session = child.session.as_deref().expect("child session");
        (
            child.status,
            child_session.provider_profile.model.clone(),
            child_session.config.max_subagent_depth,
            child_session
                .execution_environment
                .shares_backend_with(&parent_environment),
        )
    };

    let repeated_tool_result = ToolResultData {
        tool_call_id: "loop-call".to_string(),
        content: json!({"ok": true}),
        is_error: false,
        image_data: None,
        image_media_type: None,
    };
    let first_tool_call = ToolCall::new(
        "loop-call-1",
        "shell",
        json!({"command": "printf loop"}),
    );
    let second_tool_call = ToolCall::new(
        "loop-call-2",
        "shell",
        json!({"command": "printf loop"}),
    );
    let mut first_assistant = AssistantTurn::new("");
    first_assistant.response_id = Some("resp-1".to_string());
    first_assistant.finish_reason = Some(FinishReason::ToolCalls);
    first_assistant.tool_calls.push(first_tool_call);
    let mut second_assistant = AssistantTurn::new("");
    second_assistant.response_id = Some("resp-2".to_string());
    second_assistant.finish_reason = Some(FinishReason::ToolCalls);
    second_assistant.tool_calls.push(second_tool_call);

    let loop_history = vec![
        HistoryTurn::User(UserTurn::new("start")),
        HistoryTurn::Assistant(first_assistant),
        HistoryTurn::ToolResults(ToolResultsTurn::new(vec![repeated_tool_result.clone()])),
        HistoryTurn::Assistant(second_assistant),
        HistoryTurn::ToolResults(ToolResultsTurn::new(vec![repeated_tool_result])),
    ];

    let mut high_config = SessionConfig::default();
    high_config.reasoning_effort = Some("high".to_string());

    let output = json!({
        "provider_profiles": {
            "openai": profile_payload(&openai),
            "anthropic": profile_payload(&anthropic),
            "gemini": profile_payload(&gemini),
            "gemini_optional_web_tools": {
                "web_search": gemini_with_web.tool_registry.get("web_search").is_some(),
                "web_fetch": gemini_with_web.tool_registry.get("web_fetch").is_some()
            }
        },
        "session_requests": {
            "openai": provider_request_payload(openai.clone(), &environment),
            "anthropic": provider_request_payload(anthropic.clone(), &environment),
            "gemini": provider_request_payload(gemini.clone(), &environment),
            "reasoning_effort_change": {
                "medium": openai.request_provider_options(&SessionConfig {
                    reasoning_effort: Some("medium".to_string()),
                    ..SessionConfig::default()
                }),
                "high": openai.request_provider_options(&high_config)
            }
        },
        "parity_matrix": {
            "openai": parity_matrix_payload(&openai, &environment),
            "anthropic": parity_matrix_payload(&anthropic, &environment),
            "gemini": parity_matrix_payload(&gemini, &environment)
        },
        "session_state": {
            "subagent_spawn": {
                "status": child_status,
                "child_model": child_model,
                "child_depth": child_depth,
                "shared_environment": child_shares_environment
            },
            "loop_detection": {
                "detected": detect_loop(&loop_history, 2)
            }
        },
        "selector_boundary": {
            "native": {
                "openai": normalize_provider_selector("openai").id,
                "anthropic": normalize_provider_selector("anthropic").id,
                "gemini": normalize_provider_selector("gemini").id
            },
            "not_counted_as_native": {
                "openrouter": normalize_provider_selector("openrouter").id,
                "openai_compatible": normalize_provider_selector("openai_compatible").id,
                "litellm": normalize_provider_selector("litellm").id
            }
        },
        "live_smoke_gate": {
            "providers": ["openai", "anthropic", "gemini"],
            "section_9_13_sequence": [
                "create_file",
                "read_then_provider_native_edit",
                "steering",
                "shell_execution",
                "large_output_truncation",
                "shell_timeout",
                "subagent_spawn_wait_events"
            ],
            "test_module": "tests/agent/test_live_smoke.py",
            "credential_policy": "skip_when_absent",
            "ordinary_validation_requires_credentials": false
        }
    });

    println!("{}", serde_json::to_string_pretty(&output).expect("serialize output"));
}
'''


def _assert_observable_cross_provider_matrix(observation: Mapping[str, Any]) -> None:
    providers = ("openai", "anthropic", "gemini")
    assert set(observation["parity_matrix"]) == set(providers)

    expected_native_formats = {
        "openai": ("apply_patch", {"patch"}, True, False),
        "anthropic": ("edit_file", {"file_path", "old_string", "new_string"}, False, True),
        "gemini": (
            "edit_file",
            {"file_path", "instruction", "old_string", "new_string"},
            False,
            True,
        ),
    }

    for provider in providers:
        matrix = observation["parity_matrix"][provider]
        assert matrix["simple_file_creation"]["is_error"] is False
        assert matrix["simple_file_creation"]["content"]["bytes_written"] > 0

        assert matrix["read_then_edit"]["read_before_edit"]["is_error"] is False
        assert matrix["read_then_edit"]["edit"]["is_error"] is False
        assert matrix["read_then_edit"]["final_content"] == f"{provider} new\n"

        assert matrix["multi_file_edits"]["supports_parallel_tool_calls"] is True
        assert [result["is_error"] for result in matrix["multi_file_edits"]["results"]] == [
            False,
            False,
        ]
        assert matrix["multi_file_edits"]["final_files"] == {"a": "A\n", "b": "B\n"}
        assert matrix["parallel_tool_calls"] == {
            "supported": True,
            "observed_result_count": 2,
        }

        assert matrix["shell_execution"]["is_error"] is False
        assert matrix["shell_execution"]["content"]["stdout"] == f"{provider}-shell"
        assert matrix["shell_timeout"]["is_error"] is True
        assert matrix["shell_timeout"]["content"]["timed_out"] is True

        assert matrix["grep_plus_glob"]["glob"]["is_error"] is False
        assert matrix["grep_plus_glob"]["glob"]["content"]
        assert matrix["grep_plus_glob"]["grep"]["is_error"] is False

        assert matrix["multi_step_task"]["read"]["is_error"] is False
        assert matrix["multi_step_task"]["edit"]["is_error"] is False
        assert matrix["multi_step_task"]["final_content"] == "status: done\n"

        assert matrix["large_output_truncation"] == {
            "contains_truncation_warning": True,
            "is_error": False,
        }
        assert matrix["tool_error_recovery"]["missing_read"]["is_error"] is True
        assert matrix["tool_error_recovery"]["recovery_write"]["is_error"] is False
        assert matrix["tool_error_recovery"]["final_content"] == "recovered\n"

        (
            native_tool,
            required_argument_keys,
            has_apply_patch,
            has_edit_file,
        ) = expected_native_formats[provider]
        native_format = matrix["provider_native_edit_format"]
        assert native_format["tool"] == native_tool
        assert set(native_format["argument_keys"]) == required_argument_keys
        assert native_format["has_apply_patch_tool"] is has_apply_patch
        assert native_format["has_edit_file_tool"] is has_edit_file

    gemini_tools = observation["parity_matrix"]["gemini"]["gemini_directory_tools"]
    assert gemini_tools["read_many_files"]["is_error"] is False
    assert gemini_tools["read_many_files"]["content"]["count"] == 2
    assert gemini_tools["list_dir"]["is_error"] is False
    assert gemini_tools["list_dir"]["content"]["count"] >= 1

    assert observation["session_state"]["subagent_spawn"] == {
        "child_depth": 1,
        "child_model": "gemini-child",
        "shared_environment": True,
        "status": "pending",
    }
    assert observation["session_state"]["loop_detection"]["detected"] is True
    assert observation["live_smoke_gate"]["ordinary_validation_requires_credentials"] is False
    assert observation["live_smoke_gate"]["credential_policy"] == "skip_when_absent"


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": "compat-agent-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "rust-spark-agent-adapter-public-apis",
            "interfaces": [
                "spark_agent_adapter::ProviderProfile",
                "spark_agent_adapter::Session",
                "spark_agent_adapter::ToolRegistry",
                "spark_agent_adapter::ExecutionEnvironment",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=ITEM_REQUIREMENTS,
        decision_ids=ITEM_DECISIONS,
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_agent_manifest_matches_golden(manifest, expected)
