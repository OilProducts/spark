from __future__ import annotations

from pathlib import Path


def test_validation_panel_click_selects_and_focuses_graph_entities_item_7_3_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    validation_panel_text = (
        repo_root / "frontend" / "src" / "components" / "ValidationPanel.tsx"
    ).read_text(encoding="utf-8")

    required_snippets = [
        "const focusCanvasEntity = (selector: string) => {",
        "const target = document.querySelector<HTMLElement>(selector);",
        "target.tabIndex = -1;",
        "target.focus({ preventScroll: true });",
        'focusCanvasEntity(`.react-flow__node[data-id="${nodeId}"]`);',
        'focusCanvasEntity(`.react-flow__edge[data-id="${edge.id}"]`);',
        "centerOnNode(nodeId);",
        "centerOnEdge(source, target);",
    ]

    for snippet in required_snippets:
        assert snippet in validation_panel_text, f"missing diagnostic navigability snippet: {snippet}"


def test_checklist_marks_item_7_3_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.3-01]" in checklist_text


def test_validation_panel_provides_unmapped_diagnostic_fallback_item_7_3_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    validation_panel_text = (
        repo_root / "frontend" / "src" / "components" / "ValidationPanel.tsx"
    ).read_text(encoding="utf-8")

    required_snippets = [
        "const hasDirectMapping = (diag: (typeof sortedDiagnostics)[number]) => {",
        "const handleUnmappedDiagnosticFallback = () => {",
        "setSelectedNodeId(null);",
        "setSelectedEdgeId(null);",
        "focusCanvasEntity('[data-testid=\"inspector-panel\"]');",
        "No direct canvas target. Click to open graph-level review.",
        'data-testid="validation-diagnostic-fallback-hint"',
    ]

    for snippet in required_snippets:
        assert snippet in validation_panel_text, f"missing unmapped diagnostic fallback snippet: {snippet}"
