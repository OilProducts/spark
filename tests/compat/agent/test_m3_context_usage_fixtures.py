from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import textwrap
from typing import Any, Mapping

from tests.compat import harness


ITEM_ID = "M3-I4"
ITEM_REQUIREMENTS = ("REQ-008", "REQ-010", "REQ-012")
ITEM_DECISIONS = ("CD-CODING-AGENT-RUST-008", "CD-CODING-AGENT-RUST-012")


def test_m3_context_usage_fixture_matches_rust_public_api(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifest = _context_usage_manifest(tmp_path)

    _assert_fixture(
        manifest,
        compat_fixture_root / f"{manifest['fixture_id']}.json",
        compat_update_goldens,
    )


def _context_usage_manifest(tmp_path: Path) -> dict[str, Any]:
    observation = _run_rust_probe(tmp_path)
    return _manifest(
        fixture_id="agent/m3-context-usage-warning",
        scenario="m3_context_usage_warning",
        input_payload={
            "probe": "public spark-agent-adapter Session context usage API",
            "context_window_size": 100,
            "system_prompt": "sys",
            "below_threshold_user_characters": 317,
            "above_threshold_user_characters": 397,
        },
        observation=observation,
    )


def _run_rust_probe(tmp_path: Path) -> dict[str, Any]:
    repo_root = Path(__file__).resolve().parents[3]
    probe_root = tmp_path / "rust-m3-context-usage-probe"
    source_dir = probe_root / "src"
    source_dir.mkdir(parents=True)
    (probe_root / "Cargo.toml").write_text(
        textwrap.dedent(
            f"""
            [package]
            name = "spark-agent-m3-context-usage-probe"
            version = "0.0.0"
            edition = "2021"
            publish = false

            [dependencies]
            spark-agent-adapter = {{ path = {str(repo_root / "crates" / "spark-agent-adapter")!r} }}
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
use serde_json::{json, Value};
use spark_agent_adapter::{
    EventKind, ExecutionEnvironment, ProviderProfile, Session, SessionConfig, SessionEvent,
    SessionState,
};

fn state_value(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Processing => "processing",
        SessionState::AwaitingInput => "awaiting_input",
        SessionState::Closed => "closed",
    }
}

fn event_payload(event: &SessionEvent) -> Value {
    if event.kind == EventKind::UserInput {
        return json!({
            "kind": event.kind.as_str(),
            "data": {
                "content_characters": event
                    .data
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content.chars().count())
                    .unwrap_or(0)
            }
        });
    }
    json!({"kind": event.kind.as_str(), "data": event.data})
}

fn drain_events(session: &mut Session) -> Vec<Value> {
    let mut events = Vec::new();
    while let Some(event) = session.next_event() {
        events.push(event_payload(&event));
    }
    events
}

fn run_case(user_characters: usize) -> Value {
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.context_window_size = Some(100);
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.system_prompt_snapshot = "sys".to_string();
    session.next_event();

    session.submit_user_input("x".repeat(user_characters));
    let request = session.build_request(session.system_prompt_snapshot.clone());
    let request_text_characters = request
        .messages
        .iter()
        .map(|message| message.text().chars().count())
        .collect::<Vec<_>>();
    let estimate = session
        .context_usage_estimate(&request)
        .expect("context usage estimate");
    let state_before = state_value(session.state);
    let history_turns_before = session.history.len();
    let emitted = session.check_context_usage(&request);
    let state_after = state_value(session.state);
    let history_turns_after = session.history.len();

    json!({
        "emitted": emitted,
        "request_text_characters": request_text_characters,
        "estimate": estimate,
        "state_before": state_before,
        "state_after": state_after,
        "history_turns_before": history_turns_before,
        "history_turns_after": history_turns_after,
        "events": drain_events(&mut session),
    })
}

fn main() {
    let output = json!({
        "below_threshold": run_case(317),
        "above_threshold": run_case(397),
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
            "oracle": "rust-spark-agent-adapter-public-session-api",
            "interfaces": [
                "spark_agent_adapter::Session",
                "spark_agent_adapter::ProviderProfile",
                "spark_agent_adapter::ContextUsageEstimate",
                "spark_agent_adapter::SessionEvent",
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
