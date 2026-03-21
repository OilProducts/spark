from pathlib import Path

from attractor.dsl import DiagnosticSeverity, parse_dot, validate_graph


BASELINE_FIXTURES: dict[str, tuple[str, ...]] = {
    "tests/fixtures/flows/reference-1.1-03-graph-attrs.dot": (
        "stack.child_dotfile",
        "stack.child_workdir",
        "tool.hooks.pre",
        "tool.hooks.post",
    ),
    "tests/fixtures/flows/reference-1.1-03-manager-loop.dot": (
        "manager.poll_interval",
        "manager.max_cycles",
        "manager.stop_condition",
        "manager.actions",
    ),
    "tests/fixtures/reference-1.1-03-subgraph-defaults.dot": (
        "subgraph cluster_",
        "node [",
        "edge [",
    ),
    "tests/fixtures/flows/reference-1.1-03-extension-attrs.dot": (
        "ui_extension.graph_policy",
        "custom.node_behavior",
        "custom.edge_hint",
    ),
}


def test_raw_dot_baseline_fixture_set_exists_and_is_spec_valid() -> None:
    repo_root = Path(__file__).resolve().parents[3]

    for rel_path, required_snippets in BASELINE_FIXTURES.items():
        fixture_path = repo_root / rel_path
        assert fixture_path.exists(), f"Missing baseline fixture: {fixture_path}"

        fixture_text = fixture_path.read_text(encoding="utf-8")
        for snippet in required_snippets:
            assert snippet in fixture_text, f"Fixture {rel_path} missing required raw-DOT marker: {snippet}"

        graph = parse_dot(fixture_text)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in diagnostics if d.severity == DiagnosticSeverity.ERROR}
        assert error_rules == set(), f"Fixture {rel_path} has validation errors: {sorted(error_rules)}"
