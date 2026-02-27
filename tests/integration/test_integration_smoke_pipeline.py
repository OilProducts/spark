from pathlib import Path

from attractor.dsl import DiagnosticSeverity, parse_dot, validate_graph
from attractor.engine import Context, PipelineExecutor, goal_gate_satisfied, load_checkpoint
from attractor.handlers import HandlerRunner, build_default_registry


class _SmokeBackend:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str]] = []

    def run(self, node_id: str, prompt: str, context: Context, *, timeout: float | None = None) -> str:
        del context, timeout
        self.calls.append((node_id, prompt))
        return f"{node_id} completed"


def test_spec_smoke_pipeline_fixture_matches_structure() -> None:
    fixture_path = Path(__file__).resolve().parents[1] / "fixtures" / "integration_smoke_pipeline.dot"
    assert fixture_path.exists(), f"Missing smoke pipeline fixture: {fixture_path}"

    graph = parse_dot(fixture_path.read_text(encoding="utf-8"))

    goal_attr = graph.graph_attrs.get("goal")
    assert goal_attr is not None
    assert goal_attr.value == "Create a hello world Python script"
    assert set(graph.nodes.keys()) == {"start", "plan", "implement", "review", "done"}
    assert len(graph.edges) == 6

    actual_edges = {
        (
            edge.source,
            edge.target,
            str(edge.attrs["condition"].value) if "condition" in edge.attrs else None,
            str(edge.attrs["label"].value) if "label" in edge.attrs else None,
        )
        for edge in graph.edges
    }
    assert actual_edges == {
        ("start", "plan", None, None),
        ("plan", "implement", None, None),
        ("implement", "review", "outcome=success", None),
        ("implement", "plan", "outcome=fail", "Retry"),
        ("review", "done", "outcome=success", None),
        ("review", "implement", "outcome=fail", "Fix"),
    }


def test_spec_smoke_pipeline_parse_validate_execute_and_artifacts(tmp_path: Path) -> None:
    fixture_path = Path(__file__).resolve().parents[1] / "fixtures" / "integration_smoke_pipeline.dot"
    graph = parse_dot(fixture_path.read_text(encoding="utf-8"))

    assert graph.goal == "Create a hello world Python script"
    assert len(graph.nodes) == 5
    assert len(graph.edges) == 6

    diagnostics = validate_graph(graph)
    error_diagnostics = [diag for diag in diagnostics if diag.severity == DiagnosticSeverity.ERROR]
    assert error_diagnostics == []

    backend = _SmokeBackend()
    logs_root = tmp_path / "smoke-logs"
    registry = build_default_registry(codergen_backend=backend)
    result = PipelineExecutor(graph, HandlerRunner(graph, registry), logs_root=str(logs_root)).run(Context())

    assert result.status == "success"
    assert "implement" in result.completed_nodes
    assert [node_id for node_id, _ in backend.calls] == ["plan", "implement", "review"]

    for node_id in ("plan", "implement", "review"):
        stage_dir = logs_root / node_id
        assert (stage_dir / "prompt.md").is_file()
        assert (stage_dir / "response.md").is_file()
        assert (stage_dir / "status.json").is_file()

    assert goal_gate_satisfied(graph, result, "implement")

    checkpoint = load_checkpoint(logs_root / "checkpoint.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "done"
    assert "plan" in checkpoint.completed_nodes
    assert "implement" in checkpoint.completed_nodes
    assert "review" in checkpoint.completed_nodes
