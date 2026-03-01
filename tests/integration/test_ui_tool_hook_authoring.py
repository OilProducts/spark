from __future__ import annotations

from pathlib import Path


def test_graph_settings_exposes_graph_scope_tool_hook_fields_item_6_6_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    graph_settings_text = (repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx").read_text(encoding="utf-8")

    assert 'data-testid="graph-attr-input-tool_hooks.pre"' in graph_settings_text
    assert 'data-testid="graph-attr-input-tool_hooks.post"' in graph_settings_text
    assert "updateGraphAttr('tool_hooks.pre'" in graph_settings_text
    assert "updateGraphAttr('tool_hooks.post'" in graph_settings_text


def test_checklist_marks_item_6_6_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.6-01]" in checklist_text
