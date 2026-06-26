from __future__ import annotations

from collections import defaultdict
from pathlib import Path
from typing import Any, Mapping

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


ROUTE_SUCCESS_PARTIAL_DOT = """
digraph RouteFixture {
  graph [goal="Route deterministic statuses"];
  start [shape=Mdiamond];
  plan [shape=box, spark.writes_context="[\\"context.plan\\"]"];
  review [shape=box, spark.writes_context="[\\"context.reviewed\\"]"];
  done [shape=Msquare];

  start -> plan;
  plan -> review [label="needs_review", condition="preferred_label=needs_review"];
  plan -> done [label="ship"];
  review -> done;
}
"""

CONDITION_ROUTING_DOT = """
digraph ConditionFixture {
  start [shape=Mdiamond];
  decide [shape=box, spark.writes_context="[\\"context.flag\\",\\"flat.key\\"]"];
  fallback [shape=box];
  done [shape=Msquare];

  start -> decide;
  decide -> done [label="ship", weight=1, condition="preferred_label=ship && context.flag=true && flat.key=flat"];
  decide -> fallback [label="fallback", weight=2];
  fallback -> done;
}
"""

RETRY_GOAL_DOT = """
digraph RetryGoalFixture {
  graph [retry_target="implement"];
  start [shape=Mdiamond];
  implement [shape=box, max_retries=1, goal_gate=true, spark.writes_context="[\\"context.fixed\\"]"];
  done [shape=Msquare];

  start -> implement;
  implement -> done;
}
"""

CONTEXT_WRITE_DOT = """
digraph ContextContractFixture {
  graph [goal="Context contract", release="canary"];
  start [shape=Mdiamond];
  allowed [shape=box, spark.writes_context="[\\"context.keep\\",\\"context.remove\\"]"];
  denied [shape=box];
  done [shape=Msquare];

  start -> allowed;
  allowed -> denied;
  denied -> done;
}
"""

CHECKPOINT_ARTIFACT_DOT = """
digraph CheckpointFixture {
  graph [goal="Checkpoint artifacts", default_fidelity="full"];
  start [shape=Mdiamond];
  work [shape=box, prompt="Record artifact", spark.writes_context="[\\"context.work\\"]"];
  done [shape=Msquare];
  start -> work -> done;
}
"""


class _SequenceRunner:
    def __init__(self, outcomes: Mapping[str, list[Outcome] | Outcome]):
        self._outcomes: dict[str, list[Outcome]] = {}
        for node_id, value in outcomes.items():
            self._outcomes[node_id] = list(value) if isinstance(value, list) else [value]
        self.calls: list[dict[str, Any]] = []

    def __call__(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        emit_event=None,
    ) -> Outcome:
        del emit_event
        index = sum(1 for call in self.calls if call["node_id"] == node_id)
        sequence = self._outcomes.get(node_id, [Outcome(status=OutcomeStatus.SUCCESS)])
        outcome = sequence[min(index, len(sequence) - 1)]
        self.calls.append(
            {
                "node_id": node_id,
                "prompt": prompt,
                "context_before": dict(context.values),
                "outcome": harness.outcome_payload(outcome),
            }
        )
        return outcome


def test_runtime_execution_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifests = [
        _route_success_partial_manifest(tmp_path),
        _condition_routing_manifest(tmp_path),
        _retry_goal_manifest(tmp_path),
        _context_write_contract_manifest(tmp_path),
        _checkpoint_artifact_manifest(tmp_path),
    ]

    for manifest in manifests:
        _assert_runtime_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _route_success_partial_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(ROUTE_SUCCESS_PARTIAL_DOT)
    runner = _SequenceRunner(
        {
            "plan": Outcome(
                status=OutcomeStatus.PARTIAL_SUCCESS,
                preferred_label="needs_review",
                context_updates={"context.plan": "partial"},
            ),
            "review": Outcome(
                status=OutcomeStatus.SUCCESS,
                context_updates={"context.reviewed": True},
            ),
        }
    )
    events: list[dict[str, Any]] = []
    result = PipelineExecutor(
        graph,
        runner,
        logs_root=str(tmp_path / "route-run"),
        on_event=events.append,
    ).run(Context())
    return _manifest(
        fixture_id="runtime/executor-route-success-partial",
        scenario="route_success_partial",
        input_payload={"dot": ROUTE_SUCCESS_PARTIAL_DOT},
        observation={
            "result": harness.pipeline_result_payload(result),
            "runner_calls": runner.calls,
        },
        events=_stable_events(events),
    )


def _condition_routing_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(CONDITION_ROUTING_DOT)
    runner = _SequenceRunner(
        {
            "decide": Outcome(
                status=OutcomeStatus.SUCCESS,
                preferred_label="ship",
                context_updates={"context.flag": True, "flat.key": "flat"},
            )
        }
    )
    events: list[dict[str, Any]] = []
    result = PipelineExecutor(
        graph,
        runner,
        logs_root=str(tmp_path / "condition-run"),
        on_event=events.append,
    ).run(Context())
    return _manifest(
        fixture_id="runtime/executor-condition-routing",
        scenario="condition_routing",
        input_payload={"dot": CONDITION_ROUTING_DOT},
        observation={
            "result": harness.pipeline_result_payload(result),
            "runner_calls": runner.calls,
        },
        events=_stable_events(events),
    )


def _retry_goal_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(RETRY_GOAL_DOT)
    runner = _SequenceRunner(
        {
            "implement": [
                Outcome(status=OutcomeStatus.FAIL, failure_reason="try again", retryable=True),
                Outcome(
                    status=OutcomeStatus.SUCCESS,
                    context_updates={"context.fixed": True},
                ),
            ]
        }
    )
    events: list[dict[str, Any]] = []
    result = PipelineExecutor(
        graph,
        runner,
        logs_root=str(tmp_path / "retry-run"),
        on_event=events.append,
    ).run(Context())
    return _manifest(
        fixture_id="runtime/executor-retry-goal-gate",
        scenario="retry_goal_gate",
        input_payload={"dot": RETRY_GOAL_DOT},
        observation={
            "result": harness.pipeline_result_payload(result),
            "call_counts": _call_counts(runner.calls),
        },
        events=_stable_events(events),
    )


def _context_write_contract_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(CONTEXT_WRITE_DOT)
    runner = _SequenceRunner(
        {
            "allowed": Outcome(
                status=OutcomeStatus.SUCCESS,
                context_updates={"context.keep": "kept", "context.remove": None},
            ),
            "denied": Outcome(
                status=OutcomeStatus.SUCCESS,
                context_updates={"artifact_path": "/tmp/not-allowed"},
            ),
        }
    )
    events: list[dict[str, Any]] = []
    result = PipelineExecutor(
        graph,
        runner,
        logs_root=str(tmp_path / "context-run"),
        on_event=events.append,
    ).run(Context(values={"context.remove": "delete-me"}))
    return _manifest(
        fixture_id="runtime/executor-context-write-contracts",
        scenario="context_write_contracts",
        input_payload={"dot": CONTEXT_WRITE_DOT},
        observation={
            "result": harness.pipeline_result_payload(result),
            "call_counts": _call_counts(runner.calls),
        },
        events=_stable_events(events),
    )


def _checkpoint_artifact_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(CHECKPOINT_ARTIFACT_DOT)
    run_root = tmp_path / "checkpoint-run"
    runner = _SequenceRunner(
        {
            "work": Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="response body",
                context_updates={"context.work": "done"},
            )
        }
    )
    events: list[dict[str, Any]] = []
    result = PipelineExecutor(
        graph,
        runner,
        logs_root=str(run_root),
        on_event=events.append,
    ).run(Context(values={"internal.run_id": "run-checkpoint"}))
    snapshot = harness.normalize_path_tokens(
        harness.runtime_directory_snapshot(
            run_root,
            parse_json=[
                "manifest.json",
                "checkpoint.json",
                "start/status.json",
                "work/status.json",
                "done/status.json",
            ],
            read_text=["work/prompt.md", "work/response.md"],
        ),
        {"__RUN_ROOT__": run_root},
    )
    return _manifest(
        fixture_id="runtime/executor-run-directory-artifacts",
        scenario="checkpoint_artifacts",
        input_payload={"dot": CHECKPOINT_ARTIFACT_DOT},
        observation={
            "result": harness.pipeline_result_payload(result),
            "runner_calls": runner.calls,
        },
        events=_stable_events(events),
        durable_state={"run_root": snapshot},
    )


def _call_counts(calls: list[Mapping[str, Any]]) -> dict[str, int]:
    counts: defaultdict[str, int] = defaultdict(int)
    for call in calls:
        counts[str(call["node_id"])] += 1
    return dict(sorted(counts.items()))


def _stable_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    stable: list[dict[str, Any]] = []
    for event in events:
        stable_event = {
            key: value
            for key, value in event.items()
            if key not in {"duration", "delay"}
        }
        stable.append(stable_event)
    return stable


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
    events: list[dict[str, Any]] | None = None,
    durable_state: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": "compat-runtime-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I04,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-runtime-public-interfaces",
            "interfaces": [
                "attractor.engine.executor.PipelineExecutor",
                "attractor.engine.context.Context",
                "attractor.engine.outcome.Outcome",
                "durable run logs_root files",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
        "events": list(events or []),
        "durable_state": dict(durable_state or {}),
    }


def _assert_runtime_fixture(
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
    harness.assert_runtime_manifest_matches_golden(manifest, expected)
