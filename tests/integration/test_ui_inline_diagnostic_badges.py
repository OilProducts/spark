from __future__ import annotations

from pathlib import Path


def test_node_and_edge_render_inline_diagnostic_badges_item_7_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    task_node_text = (repo_root / "frontend" / "src" / "components" / "TaskNode.tsx").read_text(
        encoding="utf-8"
    )
    edge_text = (
        repo_root / "frontend" / "src" / "components" / "ValidationEdge.tsx"
    ).read_text(encoding="utf-8")

    assert 'data-testid="node-diagnostic-badge"' in task_node_text
    assert "diagnosticsForNode = nodeDiagnostics[id] || []" in task_node_text
    assert "diagnosticsCount > 0" in task_node_text
    assert 'data-testid="edge-diagnostic-badge"' in edge_text
    assert "diagnosticsForEdge.length > 0" in edge_text


def test_ui_smoke_covers_inline_node_and_edge_badges_item_7_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "inline node and edge diagnostic badges render for item 7.1-02" in ui_smoke_text
    assert "14-inline-diagnostic-badges.png" in ui_smoke_text
    assert "node-diagnostic-badge" in ui_smoke_text
    assert "edge-diagnostic-badge" in ui_smoke_text


def test_checklist_marks_item_7_1_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.1-02]" in checklist_text
