from __future__ import annotations

from pathlib import Path


def test_graph_settings_exposes_inline_help_and_precedence_notes_item_6_1_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    graph_settings_text = (repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx").read_text(encoding="utf-8")

    assert 'data-testid="graph-attrs-help"' in graph_settings_text
    assert 'data-testid="graph-attr-help-goal"' in graph_settings_text
    assert 'data-testid="graph-attr-help-label"' in graph_settings_text
    assert 'data-testid="graph-attr-help-default_max_retry"' in graph_settings_text
    assert 'data-testid="graph-attr-help-default_fidelity"' in graph_settings_text
    assert 'data-testid="graph-attr-help-model_stylesheet"' in graph_settings_text
    assert 'data-testid="graph-attr-help-retry_target"' in graph_settings_text
    assert 'data-testid="graph-attr-help-fallback_retry_target"' in graph_settings_text
    assert 'data-testid="graph-attr-help-stack.child_dotfile"' in graph_settings_text
    assert 'data-testid="graph-attr-help-stack.child_workdir"' in graph_settings_text
    assert 'data-testid="graph-attr-help-tool_hooks.pre"' in graph_settings_text
    assert 'data-testid="graph-attr-help-tool_hooks.post"' in graph_settings_text

    assert 'Graph attributes are baseline defaults. Explicit node and edge attrs win when both are set.' in graph_settings_text
    assert 'Leave blank to omit this attr from DOT output.' in graph_settings_text


def test_checklist_marks_item_6_1_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.1-03]" in checklist_text
