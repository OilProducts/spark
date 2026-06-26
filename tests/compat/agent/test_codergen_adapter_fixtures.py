from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Mapping

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.llm_runtime import RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY
from tests.compat import harness


ITEM_ID = "M3-I05-CODERGEN-AGENT-LLM-ADAPTERS"
ITEM_REQUIREMENTS = ("RR-EXE-002", "RR-EXE-006")
ITEM_DECISIONS = ("CD-RR-005", "CD-RR-006", "CD-RR-013")


class _CaptureBackend:
    def __init__(self, response: str = "backend text response", *, llm_profile: str | None = None):
        self.response = response
        self.llm_profile = llm_profile
        self.calls: list[dict[str, Any]] = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout: float | None = None,
        model: str | None = None,
        provider: str | None = None,
        llm_profile: str | None = None,
        reasoning_effort: str | None = None,
        write_contract=None,
    ) -> str:
        self.calls.append(
            {
                "node_id": node_id,
                "prompt": prompt,
                "context": dict(context.values),
                "response_contract": response_contract,
                "contract_repair_attempts": contract_repair_attempts,
                "timeout": timeout,
                "model": model,
                "provider": provider,
                "llm_profile": llm_profile,
                "reasoning_effort": reasoning_effort,
                "write_contract_allowed_keys": list(getattr(write_contract, "allowed_keys", ())),
            }
        )
        return self.response


def test_codergen_adapter_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifests = [
        _prompt_context_artifact_manifest(tmp_path),
        _simulation_manifest(tmp_path),
        _status_envelope_resolution_manifest(),
    ]

    for manifest in manifests:
        _assert_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _prompt_context_artifact_manifest(tmp_path: Path) -> dict[str, Any]:
    dot = r'''
    digraph G {
      graph [goal="Ship graph fallback"];
      task [
        shape=box,
        prompt="Plan for $goal",
        label="Label loses",
        spark.reads_context="[\"context.request.summary\",\"context.missing\"]"
      ];
    }
    '''
    graph = parse_dot(dot)
    backend = _CaptureBackend()
    logs_root = tmp_path / "codergen-logs"
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=backend),
        logs_root=logs_root,
    )
    outcome = runner(
        "task",
        "",
        Context(
            values={
                "graph.goal": "ship",
                "context.request.summary": "Ship docs safely",
                "_attractor.runtime.context_carryover": "carryover:summary:high",
            }
        ),
    )
    return _manifest(
        fixture_id="agent/codergen-prompt-context-artifacts",
        scenario="prompt_context_artifacts",
        input_payload={"dot": dot},
        observation={
            "outcome": harness.outcome_payload(outcome),
            "backend_calls": backend.calls,
            "artifacts": {
                "prompt_md": (logs_root / "task" / "prompt.md").read_text(encoding="utf-8").strip(),
                "response_md": (logs_root / "task" / "response.md").read_text(encoding="utf-8").strip(),
                "status_json": json.loads((logs_root / "task" / "status.json").read_text(encoding="utf-8")),
            },
        },
    )


def _simulation_manifest(tmp_path: Path) -> dict[str, Any]:
    dot = r'''
    digraph G {
      task [shape=box, prompt="Plan for $goal"];
    }
    '''
    graph = parse_dot(dot)
    logs_root = tmp_path / "simulation-logs"
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=None),
        logs_root=logs_root,
    )
    outcome = runner("task", "", Context(values={"graph.goal": "ship"}))
    return _manifest(
        fixture_id="agent/codergen-simulation-artifacts",
        scenario="simulation_artifacts",
        input_payload={"dot": dot},
        observation={
            "outcome": harness.outcome_payload(outcome),
            "response_md": (logs_root / "task" / "response.md").read_text(encoding="utf-8").strip(),
            "status_json": json.loads((logs_root / "task" / "status.json").read_text(encoding="utf-8")),
        },
    )


def _status_envelope_resolution_manifest() -> dict[str, Any]:
    dot = r'''
    digraph G {
      task [
        shape=box,
        prompt="Review",
        codergen.response_contract="status_envelope",
        codergen.contract_repair_attempts=2,
        spark.writes_context="[\"review.required_changes\",\"context.review.summary\"]",
        llm_provider="Anthropic"
      ];
    }
    '''
    graph = parse_dot(dot)
    backend = _CaptureBackend(llm_profile="backend-profile")
    runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend))
    outcome = runner(
        "task",
        "",
        Context(
            values={
                RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch",
                RUNTIME_LAUNCH_REASONING_EFFORT_KEY: "medium",
            }
        ),
    )
    call = dict(backend.calls[0])
    call["prompt_contains_status_contract"] = "Structured response contract:" in call.pop("prompt")
    return _manifest(
        fixture_id="agent/codergen-status-envelope-resolution",
        scenario="status_envelope_resolution",
        input_payload={"dot": dot},
        observation={
            "outcome": harness.outcome_payload(outcome),
            "backend_call": call,
        },
    )


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
            "oracle": "python-codergen-handler-public-interfaces",
            "interfaces": [
                "attractor.handlers.HandlerRunner",
                "attractor.handlers.build_default_registry",
                "attractor.handlers.builtin.CodergenHandler",
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
