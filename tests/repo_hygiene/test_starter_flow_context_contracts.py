from __future__ import annotations

import json
from pathlib import Path

from attractor.dsl import parse_dot


_STARTER_FLOWS_DIR = Path(__file__).resolve().parents[2] / "src" / "spark" / "starter_flows"


def _load_graph(relative_path: str):
    return parse_dot((_STARTER_FLOWS_DIR / relative_path).read_text(encoding="utf-8"))


def _reads_context(graph, node_id: str) -> set[str]:
    attr = graph.nodes[node_id].attrs.get("spark.reads_context")
    assert attr is not None, f"missing spark.reads_context for {node_id}"
    parsed = json.loads(str(attr.value))
    assert isinstance(parsed, list), f"expected JSON array for {node_id}"
    return {str(item) for item in parsed}


def test_parallel_review_final_review_reads_parallel_results() -> None:
    graph = _load_graph("parallel-review.dot")

    assert "parallel.results" in _reads_context(graph, "final_review")


def test_supervised_implementation_summaries_read_child_runtime_telemetry() -> None:
    graph = _load_graph("supervision/supervised-implementation.dot")
    expected_reads = {
        "context.stack.child.status",
        "context.stack.child.outcome",
        "context.stack.child.outcome_reason_code",
        "context.stack.child.outcome_reason_message",
        "context.stack.child.active_stage",
        "context.stack.child.completed_nodes",
        "context.stack.child.route_trace",
        "context.stack.child.failure_reason",
    }

    for node_id in ("summarize_failure", "summarize_success"):
        assert expected_reads <= _reads_context(graph, node_id)


def test_spec_implementation_item_state_nodes_read_active_item_binding() -> None:
    graph = _load_graph("spec-implementation/implement-milestone.dot")

    for node_id in ("mark_current_done", "validate_item_plan"):
        assert "context.item.id" in _reads_context(graph, node_id)
