from __future__ import annotations

from pathlib import Path


def test_graph_settings_uses_syntax_highlighted_stylesheet_editor_item_6_5_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    graph_settings_text = (repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx").read_text(encoding="utf-8")

    assert "<StylesheetEditor" in graph_settings_text
    assert 'data-testid="graph-model-stylesheet-editor"' in graph_settings_text

    editor_path = repo_root / "frontend" / "src" / "components" / "StylesheetEditor.tsx"
    assert editor_path.exists(), "StylesheetEditor component should exist for syntax highlighting"

    editor_text = editor_path.read_text(encoding="utf-8")
    assert 'data-testid="model-stylesheet-editor-highlight"' in editor_text
    assert "type TokenType = 'selector' | 'property' | 'value' | 'punctuation' | 'text'" in editor_text
    assert 'data-token-type={segment.type}' in editor_text


def test_checklist_marks_item_6_5_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.5-01]" in checklist_text
