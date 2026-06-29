from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import textwrap
from typing import Any, Mapping

from tests.compat import harness


ITEM_ID = "M2-006-milestone-behavior-validation"
ITEM_REQUIREMENTS = ("REQ-005", "REQ-006", "REQ-007", "REQ-008")
ITEM_DECISIONS = (
    "CD-CODING-AGENT-RUST-004",
    "CD-CODING-AGENT-RUST-005",
    "CD-CODING-AGENT-RUST-006",
)


def test_m2_rust_agent_public_behavior_matches_compat_fixture(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifest = _rust_public_behavior_manifest(tmp_path)

    _assert_fixture(
        manifest,
        compat_fixture_root / f"{manifest['fixture_id']}.json",
        compat_update_goldens,
    )


def _rust_public_behavior_manifest(tmp_path: Path) -> dict[str, Any]:
    observation = _run_rust_probe(tmp_path)
    return _manifest(
        fixture_id="agent/m2-rust-provider-tool-environment",
        scenario="m2_rust_provider_tool_environment",
        input_payload={
            "probe": "public spark-agent-adapter Rust APIs",
            "workspace": "temporary directory",
        },
        observation=observation,
    )


def _run_rust_probe(tmp_path: Path) -> dict[str, Any]:
    repo_root = Path(__file__).resolve().parents[3]
    probe_root = tmp_path / "rust-m2-probe"
    source_dir = probe_root / "src"
    source_dir.mkdir(parents=True)
    (probe_root / "Cargo.toml").write_text(
        textwrap.dedent(
            f"""
            [package]
            name = "spark-agent-m2-compat-probe"
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
    env["M2_PARENT_VISIBLE"] = "from-parent"
    env["M2_PARENT_PASSWORD"] = "secret"
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
use std::collections::BTreeMap;
use std::env;

use serde_json::{json, Value};
use spark_agent_adapter::{
    create_anthropic_profile, create_gemini_profile, create_openai_profile,
    normalize_provider_selector, CommandOptions, EnvironmentInheritancePolicy,
    ExecutionEnvironment, LocalExecutionEnvironment, ProviderProfile, SessionConfig,
    ToolDispatchContext,
};
use unified_llm_adapter::{Tool, ToolCall, ToolResult};

fn schema_property_names(profile: &ProviderProfile, tool_name: &str) -> Vec<String> {
    let mut names = profile
        .tool_registry
        .get(tool_name)
        .expect("tool definition")
        .definition
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn required_fields(profile: &ProviderProfile, tool_name: &str) -> Vec<String> {
    profile
        .tool_registry
        .get(tool_name)
        .expect("tool definition")
        .definition
        .parameters
        .get("required")
        .and_then(Value::as_array)
        .expect("required fields")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn shell_default(profile: &ProviderProfile) -> Value {
    profile
        .tool_registry
        .get("shell")
        .expect("shell tool")
        .definition
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("timeout_ms"))
        .and_then(|timeout| timeout.get("default"))
        .cloned()
        .expect("shell timeout default")
}

fn result_payload(result: ToolResult) -> Value {
    json!({
        "tool_call_id": result.tool_call_id,
        "content": result.content,
        "is_error": result.is_error
    })
}

fn main() {
    let workspace = env::var("PROBE_WORKSPACE").expect("PROBE_WORKSPACE");
    let environment = ExecutionEnvironment::local(&workspace);

    let mut config = SessionConfig::default();
    config.reasoning_effort = Some("high".to_string());

    let mut openai = create_openai_profile("gpt-5.2");
    openai
        .provider_options
        .insert("reasoning".to_string(), json!({"summary": "auto"}));
    let anthropic = create_anthropic_profile("claude-sonnet-4-5");
    let gemini = create_gemini_profile("gemini-3.1-pro-preview");

    let mut custom = create_openai_profile("gpt-5.2");
    custom.register_tool(
        Tool::passive_with_schema(
            "read_file",
            Some("custom read replacement".to_string()),
            Some(json!({"type": "object"})),
        )
        .expect("custom tool"),
    );

    let registry = openai.registry();
    let write = registry.dispatch(
        ToolCall::new(
            "call-write",
            "write_file",
            json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    let read = registry.dispatch(
        ToolCall::new(
            "call-read",
            "read_file",
            json!({"path": "notes.txt", "offset": 2, "limit": 1}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    let patch = [
        "*** Begin Patch",
        "*** Update File: notes.txt",
        "@@ alpha",
        "-alpha",
        "+gamma",
        "*** End Patch",
    ]
    .join("\n");
    let apply_patch = registry.dispatch(
        ToolCall::new("call-patch", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    let unknown = registry.dispatch(
        ToolCall::new("call-unknown", "missing_tool", json!({})),
        ToolDispatchContext::default(),
    );
    let invalid_arguments = registry.dispatch(
        ToolCall::new("call-invalid", "read_file", json!({})),
        ToolDispatchContext::default(),
    );

    let inherit_none_environment = LocalExecutionEnvironment::with_options(
        &workspace,
        10_000,
        10_000,
        EnvironmentInheritancePolicy::InheritNone,
    )
    .into_execution_environment();
    let inherit_none = inherit_none_environment
        .exec_command(
            "printf '%s|%s|%s' \"${M2_PARENT_VISIBLE-unset}\" \"${M2_PARENT_PASSWORD-unset}\" \"$EXPLICIT_VALUE\"",
            CommandOptions {
                timeout_ms: Some(1_000),
                env_vars: BTreeMap::from([(
                    "EXPLICIT_VALUE".to_string(),
                    "from-env-vars".to_string(),
                )]),
                ..CommandOptions::default()
            },
        )
        .expect("inherit-none command");

    let output = json!({
        "provider_profiles": {
            "openai": {
                "id": openai.id,
                "tool_names": openai.tool_registry.names(),
                "apply_patch_properties": schema_property_names(&openai, "apply_patch"),
                "shell_timeout_default": shell_default(&openai),
                "provider_options": openai.request_provider_options(&config),
                "supports_reasoning": openai.supports("reasoning"),
                "supports_streaming": openai.supports("streaming"),
                "supports_parallel_tool_calls": openai.supports("parallel_tool_calls"),
                "context_window_size": openai.context_window_size,
            },
            "anthropic": {
                "id": anthropic.id,
                "tool_names": anthropic.tool_registry.names(),
                "has_apply_patch": anthropic.tool_registry.get("apply_patch").is_some(),
                "edit_file_required": required_fields(&anthropic, "edit_file"),
                "edit_file_properties": schema_property_names(&anthropic, "edit_file"),
                "shell_timeout_default": shell_default(&anthropic),
                "context_window_size": anthropic.context_window_size,
            },
            "gemini": {
                "id": gemini.id,
                "tool_names": gemini.tool_registry.names(),
                "edit_file_properties": schema_property_names(&gemini, "edit_file"),
                "read_many_files_required": required_fields(&gemini, "read_many_files"),
                "context_window_size": gemini.context_window_size,
            }
        },
        "selector_normalization": {
            "codex": normalize_provider_selector("codex").id,
            "openai-compatible": normalize_provider_selector("openai-compatible").id,
            "claude_code": normalize_provider_selector("claude_code").id,
            "google-gemini": normalize_provider_selector("google-gemini").id,
        },
        "custom_latest_wins": {
            "read_file_description": custom
                .tool_registry
                .get("read_file")
                .expect("custom read_file")
                .definition
                .description,
            "read_file_count": custom
                .tool_registry
                .names()
                .into_iter()
                .filter(|name| name == "read_file")
                .count(),
        },
        "tool_dispatch": {
            "write_file": result_payload(write),
            "read_file": result_payload(read),
            "apply_patch": result_payload(apply_patch),
            "unknown_tool": result_payload(unknown),
            "invalid_arguments": result_payload(invalid_arguments),
            "final_file": environment
                .read_file("notes.txt", None, None)
                .expect("final file"),
        },
        "local_environment": {
            "inherit_none_stdout": inherit_none.stdout,
            "inherit_none_exit_code": inherit_none.exit_code,
            "inherit_none_timed_out": inherit_none.timed_out,
        }
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
            "oracle": "rust-spark-agent-adapter-public-apis",
            "interfaces": [
                "spark_agent_adapter::ProviderProfile",
                "spark_agent_adapter::ToolRegistry",
                "spark_agent_adapter::ExecutionEnvironment",
                "spark_agent_adapter::LocalExecutionEnvironment",
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
