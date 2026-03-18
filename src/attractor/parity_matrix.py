from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import json
from pathlib import Path
import tempfile
from typing import Callable, Dict, List

from attractor.dsl import DiagnosticSeverity, parse_dot, validate_graph
from attractor.engine import Context, PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.engine.routing import select_next_edge
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.registry import HandlerRegistry
from attractor.interviewer import Answer, QueueInterviewer
from attractor.transforms import GoalVariableTransform, ModelStylesheetTransform, TransformPipeline


CROSS_FEATURE_PARITY_MATRIX_ROWS = [
    "Parse a simple linear pipeline (start -> A -> B -> done)",
    "Parse a pipeline with graph-level attributes (goal, label)",
    "Parse multi-line node attributes",
    "Validate: missing start node -> error",
    "Validate: missing exit node -> error",
    "Validate: orphan node -> warning",
    "Execute a linear 3-node pipeline end-to-end",
    "Execute with conditional branching (success/fail paths)",
    "Execute with retry on failure (max_retries=2)",
    "Goal gate blocks exit when unsatisfied",
    "Goal gate allows exit when all satisfied",
    "Wait.human presents choices and routes on selection",
    "Edge selection: condition match wins over weight",
    "Edge selection: weight breaks ties for unconditional edges",
    "Edge selection: lexical tiebreak as final fallback",
    "Context updates from one node are visible to the next",
    "Checkpoint save and resume produces same result",
    "Stylesheet applies model override to nodes by shape",
    "Prompt variable expansion ($goal) works",
    "Parallel fan-out and fan-in complete correctly",
    "Custom handler registration and execution works",
    "Pipeline with 10+ nodes completes without errors",
]


@dataclass(frozen=True)
class ParityMatrixCase:
    name: str
    check: Callable[[], bool]


class _Backend:
    def __init__(self, plan: Dict[str, bool]):
        self.plan = plan
        self.calls: List[str] = []

    def run(self, node_id, prompt, context, *, timeout=None):  # noqa: ANN001, ANN201
        self.calls.append(node_id)
        return bool(self.plan.get(node_id, True))


class _FlakyHandler:
    def __init__(self) -> None:
        self.calls = 0

    def run(self, runtime):  # noqa: ANN001, ANN201
        self.calls += 1
        if self.calls <= 2:
            return Outcome(status=OutcomeStatus.RETRY, failure_reason="temporary")
        return Outcome(status=OutcomeStatus.SUCCESS)


class _ContextWriterHandler:
    def run(self, runtime):  # noqa: ANN001, ANN201
        return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"artifact_path": "/tmp/out"})


class _ContextReaderHandler:
    def run(self, runtime):  # noqa: ANN001, ANN201
        return Outcome(
            status=OutcomeStatus.SUCCESS
            if runtime.context.get("artifact_path", "") == "/tmp/out"
            else OutcomeStatus.FAIL,
            failure_reason="" if runtime.context.get("artifact_path", "") == "/tmp/out" else "missing context",
        )


class _CustomHandler:
    def run(self, runtime):  # noqa: ANN001, ANN201
        return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"custom.handler.ran": "true"})


def run_cross_feature_parity_matrix(report_path: Path | str) -> Dict[str, object]:
    path = Path(report_path)
    rows: List[Dict[str, object]] = []

    for case in _cross_feature_parity_cases():
        try:
            passed = bool(case.check())
            evidence = "check returned true" if passed else "check returned false"
        except Exception as exc:  # noqa: BLE001
            passed = False
            evidence = f"{type(exc).__name__}: {exc}"
        rows.append({"name": case.name, "pass": passed, "evidence": evidence})

    summary = {
        "total": len(rows),
        "passed": sum(1 for row in rows if bool(row["pass"])),
        "failed": sum(1 for row in rows if not bool(row["pass"])),
    }
    report = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "summary": summary,
        "rows": rows,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def enforce_cross_feature_parity_release_gate(report: Dict[str, object]) -> None:
    rows = report.get("rows")
    if not isinstance(rows, list):
        raise RuntimeError("Cross-feature parity matrix release gate failed: unchecked rows")

    row_pass_by_name: Dict[str, bool] = {}
    for row in rows:
        if not isinstance(row, dict):
            continue
        name = row.get("name")
        if not isinstance(name, str):
            continue
        row_pass_by_name[name] = bool(row.get("pass", False))

    unchecked = [name for name in CROSS_FEATURE_PARITY_MATRIX_ROWS if not row_pass_by_name.get(name, False)]
    if unchecked:
        joined = ", ".join(unchecked)
        raise RuntimeError(f"Cross-feature parity matrix release gate failed: unchecked rows: {joined}")


def _cross_feature_parity_cases() -> List[ParityMatrixCase]:
    cases = [
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[0], _check_parse_simple_linear_pipeline),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[1], _check_parse_graph_attributes),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[2], _check_parse_multiline_node_attributes),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[3], _check_validate_missing_start_node),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[4], _check_validate_missing_exit_node),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[5], _check_validate_orphan_node_warning),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[6], _check_execute_linear_pipeline),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[7], _check_execute_conditional_success_fail),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[8], _check_retry_max_retries_two),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[9], _check_goal_gate_blocks_exit),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[10], _check_goal_gate_allows_exit),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[11], _check_wait_human_routes_on_selection),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[12], _check_edge_condition_over_weight),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[13], _check_edge_weight_for_unconditional_edges),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[14], _check_edge_lexical_tiebreak),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[15], _check_context_updates_visible_to_next),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[16], _check_checkpoint_resume_same_result),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[17], _check_stylesheet_model_override),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[18], _check_prompt_goal_variable_expansion),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[19], _check_parallel_fan_out_and_fan_in),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[20], _check_custom_handler_registration),
        ParityMatrixCase(CROSS_FEATURE_PARITY_MATRIX_ROWS[21], _check_pipeline_with_ten_nodes),
    ]
    return cases


def _check_parse_simple_linear_pipeline() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            A [shape=box]
            B [shape=box]
            done [shape=Msquare]
            start -> A -> B -> done
        }
        """
    )
    return len(graph.nodes) == 4 and len(graph.edges) == 3


def _check_parse_graph_attributes() -> bool:
    graph = parse_dot(
        """
        digraph G {
            graph [goal="Ship release", label="Release Flow"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
    )
    goal = graph.graph_attrs.get("goal")
    label = graph.graph_attrs.get("label")
    return bool(goal and goal.value == "Ship release" and label and label.value == "Release Flow")


def _check_parse_multiline_node_attributes() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [
                shape=box,
                prompt="run checks",
                max_retries=2
            ]
            done [shape=Msquare]
            start -> task -> done
        }
        """
    )
    task = graph.nodes["task"]
    return bool(
        task.attrs["shape"].value == "box"
        and task.attrs["prompt"].value == "run checks"
        and task.attrs["max_retries"].value == 2
    )


def _check_validate_missing_start_node() -> bool:
    graph = parse_dot(
        """
        digraph G {
            task [shape=box]
            done [shape=Msquare]
            task -> done
        }
        """
    )
    diagnostics = validate_graph(graph)
    return any(d.rule_id == "start_node" and d.severity == DiagnosticSeverity.ERROR for d in diagnostics)


def _check_validate_missing_exit_node() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            start -> task
        }
        """
    )
    diagnostics = validate_graph(graph)
    return any(d.rule_id == "terminal_node" and d.severity == DiagnosticSeverity.ERROR for d in diagnostics)


def _check_validate_orphan_node_warning() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            done [shape=Msquare]
            orphan [shape=box]
            start -> task -> done
        }
        """
    )
    diagnostics = validate_graph(graph)
    return any(d.rule_id == "reachability" and d.severity == DiagnosticSeverity.WARNING for d in diagnostics)


def _check_execute_linear_pipeline() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box]
            implement [shape=box]
            done [shape=Msquare]
            start -> plan -> implement -> done
        }
        """
    )
    backend = _Backend({"start": True, "plan": True, "implement": True})
    registry = build_default_registry(codergen_backend=backend)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and result.completed_nodes == ["start", "plan", "implement"]


def _check_execute_conditional_success_fail() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            gate [shape=box]
            fix [shape=box]
            done [shape=Msquare]
            start -> gate
            gate -> done [condition="outcome=success"]
            gate -> fix [condition="outcome=fail"]
            fix -> done
        }
        """
    )
    backend = _Backend({"start": True, "gate": False, "fix": True})
    registry = build_default_registry(codergen_backend=backend)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and "fix" in result.completed_nodes


def _check_retry_max_retries_two() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            flaky [shape=box, type="flaky", max_retries=2]
            done [shape=Msquare]
            start -> flaky -> done
        }
        """
    )
    registry = HandlerRegistry()
    registry.handlers = build_default_registry(codergen_backend=_Backend({})).handlers.copy()
    flaky = _FlakyHandler()
    registry.register("flaky", flaky)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and flaky.calls == 3


def _check_goal_gate_blocks_exit() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            implement [shape=box, goal_gate=true, max_retries=0]
            done [shape=Msquare]
            start -> implement -> done
        }
        """
    )
    backend = _Backend({"start": True, "implement": False})
    registry = build_default_registry(codergen_backend=backend)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "fail" and "Goal gate unsatisfied" in result.failure_reason


def _check_goal_gate_allows_exit() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            implement [shape=box, goal_gate=true]
            done [shape=Msquare]
            start -> implement -> done
        }
        """
    )
    backend = _Backend({"start": True, "implement": True})
    registry = build_default_registry(codergen_backend=backend)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success"


def _check_wait_human_routes_on_selection() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon]
            ship [shape=box]
            fix [shape=box]
            done [shape=Msquare]
            start -> gate
            gate -> ship [label="Approve"]
            gate -> fix [label="Fix"]
            ship -> done
            fix -> done
        }
        """
    )
    interviewer = QueueInterviewer([Answer(selected_values=["Fix"])])
    registry = build_default_registry(codergen_backend=_Backend({"start": True, "fix": True}), interviewer=interviewer)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and "fix" in result.completed_nodes


def _check_edge_condition_over_weight() -> bool:
    graph = parse_dot(
        """
        digraph G {
            a [shape=box]
            b [shape=box]
            c [shape=box]
            a -> b [condition="outcome=success", weight=0]
            a -> c [weight=10]
        }
        """
    )
    outcome = Outcome(status=OutcomeStatus.SUCCESS)
    edges = [edge for edge in graph.edges if edge.source == "a"]
    selected = select_next_edge(edges, outcome, Context())
    return bool(selected and selected.target == "b")


def _check_edge_weight_for_unconditional_edges() -> bool:
    graph = parse_dot(
        """
        digraph G {
            a [shape=box]
            high [shape=box]
            low [shape=box]
            a -> high [weight=10]
            a -> low [weight=1]
        }
        """
    )
    selected = select_next_edge([edge for edge in graph.edges if edge.source == "a"], Outcome(OutcomeStatus.SUCCESS), Context())
    return bool(selected and selected.target == "high")


def _check_edge_lexical_tiebreak() -> bool:
    graph = parse_dot(
        """
        digraph G {
            a [shape=box]
            alpha [shape=box]
            beta [shape=box]
            a -> beta [weight=5]
            a -> alpha [weight=5]
        }
        """
    )
    selected = select_next_edge([edge for edge in graph.edges if edge.source == "a"], Outcome(OutcomeStatus.SUCCESS), Context())
    return bool(selected and selected.target == "alpha")


def _check_context_updates_visible_to_next() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            writer [shape=box, type="ctx.write"]
            reader [shape=box, type="ctx.read"]
            done [shape=Msquare]
            start -> writer -> reader -> done
        }
        """
    )
    registry = HandlerRegistry()
    registry.handlers = build_default_registry(codergen_backend=_Backend({})).handlers.copy()
    registry.register("ctx.write", _ContextWriterHandler())
    registry.register("ctx.read", _ContextReaderHandler())
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success"


def _check_checkpoint_resume_same_result() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box]
            review [shape=box]
            done [shape=Msquare]
            start -> plan -> review -> done
        }
        """
    )

    def runner(node_id: str, prompt: str, context: Context) -> Outcome:
        return Outcome(status=OutcomeStatus.SUCCESS)

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        resumed_executor = PipelineExecutor(
            graph,
            runner,
            logs_root=str(root / "resume-logs"),
            checkpoint_file=str(root / "resume-checkpoint.json"),
        )
        paused = resumed_executor.run(Context(), max_steps=1)
        resumed = resumed_executor.run(Context(), resume=True)

        full = PipelineExecutor(
            graph,
            runner,
            logs_root=str(root / "full-logs"),
            checkpoint_file=str(root / "full-checkpoint.json"),
        ).run(Context())

    return bool(
        paused.status == "paused"
        and resumed.status == "success"
        and resumed.status == full.status
        and resumed.current_node == full.current_node
        and resumed.completed_nodes == full.completed_nodes
    )


def _check_stylesheet_model_override() -> bool:
    graph = parse_dot(
        """
        digraph G {
            graph [model_stylesheet=".box { llm_model: gpt-5; }"]
            start [shape=Mdiamond]
            task [shape=box, class="box"]
            gate [shape=hexagon]
            done [shape=Msquare]
            start -> task -> gate -> done
        }
        """
    )
    pipeline = TransformPipeline()
    pipeline.register(ModelStylesheetTransform())
    graph = pipeline.apply(graph)

    task_attr = graph.nodes["task"].attrs.get("llm_model")
    gate_attr = graph.nodes["gate"].attrs.get("llm_model")
    return bool(task_attr and task_attr.value == "gpt-5" and gate_attr and gate_attr.value != "gpt-5")


def _check_prompt_goal_variable_expansion() -> bool:
    graph = parse_dot(
        """
        digraph G {
            graph [goal="Ship docs"]
            start [shape=Mdiamond]
            task [shape=box, prompt="Build $goal"]
            done [shape=Msquare]
            start -> task -> done
        }
        """
    )
    transformed = GoalVariableTransform().apply(graph)
    return transformed.nodes["task"].attrs["prompt"].value == "Build Ship docs"


def _check_parallel_fan_out_and_fan_in() -> bool:
    flow_path = Path(__file__).resolve().parents[2] / "starter-flows" / "parallel-review.dot"
    graph = parse_dot(flow_path.read_text(encoding="utf-8"))
    interviewer = QueueInterviewer([Answer(selected_values=["Proceed"])])
    backend = _Backend(
        {
            "start": True,
            "plan": True,
            "implement": True,
            "branch_docs": True,
            "branch_tests": True,
            "final_review": True,
        }
    )
    registry = build_default_registry(codergen_backend=backend, interviewer=interviewer)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    parallel_results = result.context.get("parallel.results", [])
    branch_ids = {entry.get("id", "") for entry in parallel_results if isinstance(entry, dict)}
    return bool(result.status == "success" and {"branch_docs", "branch_tests"}.issubset(branch_ids))


def _check_custom_handler_registration() -> bool:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            custom [shape=box, type="custom.handler"]
            done [shape=Msquare]
            start -> custom -> done
        }
        """
    )
    registry = HandlerRegistry()
    registry.handlers = build_default_registry(codergen_backend=_Backend({})).handlers.copy()
    registry.register("custom.handler", _CustomHandler())
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and result.context.get("custom.handler.ran", "") == "true"


def _check_pipeline_with_ten_nodes() -> bool:
    parts = ["digraph G {", "start [shape=Mdiamond]"]
    for i in range(1, 11):
        parts.append(f"n{i} [shape=box]")
    parts.append("done [shape=Msquare]")
    parts.append("start -> n1")
    for i in range(1, 10):
        parts.append(f"n{i} -> n{i+1}")
    parts.append("n10 -> done")
    parts.append("}")
    graph = parse_dot("\n".join(parts))

    registry = build_default_registry(codergen_backend=_Backend({}))
    result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())
    return result.status == "success" and len(result.completed_nodes) == 11
