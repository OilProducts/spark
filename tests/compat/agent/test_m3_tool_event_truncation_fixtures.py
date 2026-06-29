from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import textwrap
from typing import Any, Mapping

from tests.compat import harness


ITEM_ID = "M3-I2"
ITEM_REQUIREMENTS = ("REQ-004", "REQ-006", "REQ-009", "REQ-012")
ITEM_DECISIONS = ("CD-CODING-AGENT-RUST-007", "CD-CODING-AGENT-RUST-012")


def test_m3_tool_event_truncation_fixture_matches_rust_public_api(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifest = _tool_event_truncation_manifest(tmp_path)

    _assert_fixture(
        manifest,
        compat_fixture_root / f"{manifest['fixture_id']}.json",
        compat_update_goldens,
    )


def _tool_event_truncation_manifest(tmp_path: Path) -> dict[str, Any]:
    observation = _run_rust_probe(tmp_path)
    return _manifest(
        fixture_id="agent/m3-tool-event-truncation",
        scenario="m3_tool_event_truncation",
        input_payload={
            "probe": "public spark-agent-adapter ToolRegistry dispatch API",
            "success_tool": "shell",
            "error_tool": "grep",
        },
        observation=observation,
    )


def _run_rust_probe(tmp_path: Path) -> dict[str, Any]:
    repo_root = Path(__file__).resolve().parents[3]
    probe_root = tmp_path / "rust-m3-tool-event-probe"
    source_dir = probe_root / "src"
    source_dir.mkdir(parents=True)
    (probe_root / "Cargo.toml").write_text(
        textwrap.dedent(
            f"""
            [package]
            name = "spark-agent-m3-tool-event-probe"
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
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use spark_agent_adapter::{
    RegisteredTool, SessionConfig, ToolDefinition, ToolDispatchContext, ToolDispatchEvent,
    ToolExecutionOutput, ToolRegistry,
};
use unified_llm_adapter::{ToolCall, ToolResult};

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition::new(name, "Probe tool", Some(json!({"type": "object"})))
        .expect("valid tool definition")
}

fn result_payload(result: &ToolResult) -> Value {
    json!({
        "tool_call_id": result.tool_call_id,
        "content": result.content,
        "is_error": result.is_error
    })
}

fn event_payloads(events: &[ToolDispatchEvent]) -> Value {
    Value::Array(
        events
            .iter()
            .map(|event| json!({"kind": event.kind, "data": event.data}))
            .collect(),
    )
}

fn dispatch_with_events(
    registry: &ToolRegistry,
    tool_call: ToolCall,
    config: SessionConfig,
) -> (ToolResult, Vec<ToolDispatchEvent>) {
    let events = Arc::new(Mutex::new(Vec::<ToolDispatchEvent>::new()));
    let captured_events = events.clone();
    let result = registry.dispatch(
        tool_call,
        ToolDispatchContext {
            config,
            event_hook: Some(Arc::new(move |event| {
                captured_events.lock().expect("events").push(event);
            })),
            ..ToolDispatchContext::default()
        },
    );
    let events = events.lock().expect("events").clone();
    (result, events)
}

fn main() {
    let full_success =
        "SUCCESS-START\nabcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\nSUCCESS-END"
            .to_string();
    let full_error =
        "ERROR-START\n0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\nERROR-END"
            .to_string();

    let mut registry = ToolRegistry::new();
    let success_for_executor = full_success.clone();
    registry.register(RegisteredTool::new_with_executor(
        definition("shell"),
        Arc::new(move |_| Ok(ToolExecutionOutput::success(success_for_executor.clone()))),
    ));
    let error_for_executor = full_error.clone();
    registry.register(RegisteredTool::new_with_executor(
        definition("grep"),
        Arc::new(move |_| {
            Ok(ToolExecutionOutput::error(json!({
                "stderr": error_for_executor,
                "exit_code": 2
            })))
        }),
    ));

    let mut config = SessionConfig::default();
    config.tool_output_limits.insert("shell".to_string(), 24);
    config.tool_output_limits.insert("grep".to_string(), 24);

    let (success, success_events) = dispatch_with_events(
        &registry,
        ToolCall::new("call-success", "shell", json!({})),
        config.clone(),
    );
    let (recoverable_error, error_events) = dispatch_with_events(
        &registry,
        ToolCall::new("call-error", "grep", json!({})),
        config,
    );

    let output = json!({
        "success": {
            "full_output": full_success,
            "result": result_payload(&success),
            "events": event_payloads(&success_events),
            "result_has_truncation_marker": success
                .content
                .as_str()
                .map(|content| content.contains("[WARNING: Tool output was truncated."))
                .unwrap_or(false),
            "event_output_matches_full_output": success_events
                .last()
                .and_then(|event| event.data.get("output"))
                == Some(&json!(full_success)),
        },
        "recoverable_error": {
            "full_error": {
                "stderr": full_error,
                "exit_code": 2
            },
            "result": result_payload(&recoverable_error),
            "events": event_payloads(&error_events),
            "result_has_truncation_marker": recoverable_error
                .content
                .as_str()
                .map(|content| content.contains("[WARNING: Tool output was truncated."))
                .unwrap_or(false),
            "event_error_matches_full_error": error_events
                .last()
                .and_then(|event| event.data.get("error"))
                == Some(&json!({"stderr": full_error, "exit_code": 2})),
        },
    });

    println!("{}", serde_json::to_string_pretty(&output).expect("serialize output"));
}
'''


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
            "oracle": "rust-spark-agent-adapter-public-dispatch-api",
            "interfaces": [
                "spark_agent_adapter::ToolRegistry",
                "spark_agent_adapter::ToolDispatchContext",
                "spark_agent_adapter::ToolDispatchEvent",
                "unified_llm_adapter::ToolResult",
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
