from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import ChildRunRequest, ChildRunResult
from attractor.interviewer import Answer, QueueInterviewer
from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


NOOP_DOT = """
digraph HandlerNoop {
  start [shape=Mdiamond];
  gate [shape=diamond];
  done [shape=Msquare];
  start -> gate -> done;
}
"""

TOOL_SUCCESS_DOT = """
digraph ToolSuccess {
  tool_node [
    shape=parallelogram,
    tool.command="printf compat-tool",
    tool.artifacts.stdout="tool/stdout.txt"
  ];
}
"""

TOOL_PREHOOK_FAILURE_DOT = """
digraph ToolPrehook {
  graph [tool.hooks.pre="false"];
  tool_node [shape=parallelogram, tool.command="printf should-not-run"];
}
"""

WAIT_HUMAN_DOT = """
digraph HumanGate {
  gate [shape=hexagon, prompt="Choose"];
  ship [shape=box];
  fix [shape=box];
  gate -> ship [label="[S] Ship"];
  gate -> fix [label="[F] Fix"];
}
"""

PARALLEL_DOT = """
digraph ParallelFixture {
  fan [shape=component, max_parallel=2];
  a [shape=box, type="custom.success"];
  b [shape=box, type="custom.fail"];
  a_stop [shape=tripleoctagon];
  b_stop [shape=tripleoctagon];

  fan -> a;
  fan -> b;
  a -> a_stop [condition="outcome=success"];
  b -> b_stop [condition="outcome=fail"];
}
"""

FAN_IN_DOT = """
digraph FanInFixture {
  fan_in [shape=tripleoctagon];
}
"""


class _StubBackend:
    def run(self, *args, **kwargs) -> bool:
        del args, kwargs
        return True


class _SuccessHandler:
    thread_safe = True

    def execute(self, runtime):
        del runtime
        return Outcome(status=OutcomeStatus.SUCCESS, notes="custom-success")


class _FailHandler:
    thread_safe = True

    def execute(self, runtime):
        del runtime
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="custom-fail", retryable=False)


def test_handler_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifests = [
        _noop_manifest(),
        _tool_success_manifest(tmp_path),
        _tool_prehook_failure_manifest(tmp_path),
        _wait_human_manifest(),
        _parallel_fan_in_manifest(),
        _manager_loop_child_manifest(tmp_path),
    ]

    for manifest in manifests:
        _assert_runtime_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _noop_manifest() -> dict[str, Any]:
    graph = parse_dot(NOOP_DOT)
    runner = HandlerRunner(graph, build_default_registry(codergen_backend=_StubBackend()))
    context = Context()
    observations = {
        node_id: harness.outcome_payload(runner(node_id, "", context))
        for node_id in ("start", "gate", "done")
    }
    return _manifest(
        fixture_id="runtime/handler-start-exit-conditional",
        scenario="handler_noop_start_exit_conditional",
        input_payload={"dot": NOOP_DOT},
        observation={"outcomes": observations},
    )


def _tool_success_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(TOOL_SUCCESS_DOT)
    logs_root = tmp_path / "tool-success-logs"
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=_StubBackend()),
        logs_root=logs_root,
    )
    context = Context(values={"internal.run_workdir": str(tmp_path)})
    outcome = runner("tool_node", "", context)
    snapshot = harness.normalize_path_tokens(
        harness.runtime_directory_snapshot(
            logs_root,
            read_text=["tool_node/tool_output.txt"],
        ),
        {"__LOGS_ROOT__": logs_root, "__TMP__": tmp_path},
    )
    artifact_snapshot = harness.normalize_path_tokens(
        harness.runtime_directory_snapshot(
            logs_root.parent / "artifacts",
            read_text=["tool_node/tool/stdout.txt"],
        ),
        {"__ARTIFACT_ROOT__": logs_root.parent / "artifacts", "__TMP__": tmp_path},
    )
    return _manifest(
        fixture_id="runtime/handler-tool-success-artifacts",
        scenario="handler_tool_success_artifacts",
        input_payload={"dot": TOOL_SUCCESS_DOT},
        observation={
            "outcome": harness.outcome_payload(outcome),
            "context_after_merge": harness.normalize_path_tokens(
                _context_after(outcome, context),
                {"__TMP__": tmp_path},
            ),
        },
        durable_state={"logs_root": snapshot, "artifacts": artifact_snapshot},
    )


def _tool_prehook_failure_manifest(tmp_path: Path) -> dict[str, Any]:
    graph = parse_dot(TOOL_PREHOOK_FAILURE_DOT)
    logs_root = tmp_path / "tool-prehook-logs"
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=_StubBackend()),
        logs_root=logs_root,
    )
    outcome = runner("tool_node", "", Context(values={"internal.run_workdir": str(tmp_path)}))
    snapshot = harness.normalize_path_tokens(
        harness.runtime_directory_snapshot(
            logs_root,
            parse_jsonl=["tool_node/tool_hook_failures.jsonl"],
            read_text=["tool_node/tool_output.txt"],
        ),
        {"__LOGS_ROOT__": logs_root, "__TMP__": tmp_path},
    )
    return _manifest(
        fixture_id="runtime/handler-tool-prehook-failure",
        scenario="handler_tool_prehook_failure",
        input_payload={"dot": TOOL_PREHOOK_FAILURE_DOT},
        observation={"outcome": harness.outcome_payload(outcome)},
        durable_state={"logs_root": snapshot},
    )


def _wait_human_manifest() -> dict[str, Any]:
    graph = parse_dot(WAIT_HUMAN_DOT)
    runner = HandlerRunner(
        graph,
        build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([Answer(selected_values=["ship"])]),
        ),
    )
    outcome = runner("gate", "Choose", Context())
    skipped_runner = HandlerRunner(
        graph,
        build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([]),
        ),
    )
    skipped = skipped_runner("gate", "Choose", Context())
    return _manifest(
        fixture_id="runtime/handler-wait-human-answer",
        scenario="handler_wait_human_answer",
        input_payload={"dot": WAIT_HUMAN_DOT},
        observation={
            "selected": harness.outcome_payload(outcome),
            "skipped": harness.outcome_payload(skipped),
        },
    )


def _parallel_fan_in_manifest() -> dict[str, Any]:
    parallel_graph = parse_dot(PARALLEL_DOT)
    registry = build_default_registry(
        codergen_backend=_StubBackend(),
        extra_handlers={
            "custom.success": _SuccessHandler(),
            "custom.fail": _FailHandler(),
        },
    )
    parallel_runner = HandlerRunner(parallel_graph, registry)
    parallel_outcome = parallel_runner("fan", "", Context())

    fan_in_graph = parse_dot(FAN_IN_DOT)
    fan_in_runner = HandlerRunner(
        fan_in_graph,
        build_default_registry(codergen_backend=_StubBackend()),
    )
    fan_in_outcome = fan_in_runner(
        "fan_in",
        "",
        Context(values={"parallel.results": parallel_outcome.context_updates["parallel.results"]}),
    )
    return _manifest(
        fixture_id="runtime/handler-parallel-fanout-join",
        scenario="handler_parallel_and_fan_in",
        input_payload={"parallel_dot": PARALLEL_DOT, "fan_in_dot": FAN_IN_DOT},
        observation={
            "parallel": harness.outcome_payload(parallel_outcome),
            "fan_in": harness.outcome_payload(fan_in_outcome),
        },
    )


def _manager_loop_child_manifest(tmp_path: Path) -> dict[str, Any]:
    child_dot_path = tmp_path / "child.dot"
    child_workdir = tmp_path / "child-workdir"
    child_workdir.mkdir(parents=True, exist_ok=True)
    child_dot_path.write_text(
        """
digraph Child {
  start [shape=Mdiamond];
  task [shape=box, prompt="Child task"];
  done [shape=Msquare];
  start -> task -> done;
}
""",
        encoding="utf-8",
    )
    parent_dot = f"""
digraph ManagerFixture {{
  graph [stack.child_dotfile="{child_dot_path}", stack.child_workdir="{child_workdir}"];
  manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""];
}}
"""
    graph = parse_dot(parent_dot)
    requests: list[dict[str, Any]] = []

    def launch_child(request: ChildRunRequest) -> ChildRunResult:
        requests.append(
            {
                "child_run_id": request.child_run_id,
                "child_flow_name": request.child_flow_name,
                "child_flow_path": str(request.child_flow_path),
                "child_workdir": str(request.child_workdir),
                "parent_node_id": request.parent_node_id,
                "parent_run_id": request.parent_run_id,
                "root_run_id": request.root_run_id,
            }
        )
        return ChildRunResult(
            run_id=request.child_run_id,
            status="completed",
            outcome="success",
            current_node="done",
            completed_nodes=["start", "task", "done"],
            route_trace=["start", "task", "done"],
        )

    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=_StubBackend()),
        logs_root=tmp_path / "manager-logs",
        child_run_launcher=launch_child,
    )
    outcome = runner(
        "manager",
        "",
        Context(values={"internal.run_id": "run-parent", "context.milestone.id": "M0"}),
    )
    return _manifest(
        fixture_id="runtime/handler-manager-loop-child",
        scenario="handler_manager_loop_child",
        input_payload=harness.normalize_path_tokens(
            {"dot": parent_dot},
            {"__TMP__": tmp_path},
        ),
        observation={
            "outcome": harness.outcome_payload(outcome),
            "child_requests": harness.normalize_path_tokens(
                requests,
                {"__TMP__": tmp_path},
            ),
        },
    )


def _context_after(outcome: Any, context: Context) -> dict[str, Any]:
    cloned = context.clone()
    cloned.merge_updates(outcome.context_updates)
    return dict(cloned.values)


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
    durable_state: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": "compat-runtime-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I04,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-handler-public-interfaces",
            "interfaces": [
                "attractor.handlers.HandlerRunner",
                "built-in handler protocol",
                "deterministic fake backend/interviewer/child launcher",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
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
