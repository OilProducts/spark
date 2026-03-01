from __future__ import annotations

import asyncio
from pathlib import Path

import attractor.api.server as server


INVALID_STYLESHEET_FLOW = '''
digraph stylesheet_probe {
    graph [model_stylesheet=".bad$class { llm_model: gpt-5; }"];
    start [label="Start", shape=Mdiamond];
    done [label="Done", shape=Msquare];
    start -> done;
}
'''.strip()

WHITESPACE_STYLESHEET_FLOW = '''
digraph stylesheet_probe_whitespace {
    graph [model_stylesheet="   "];
    start [label="Start", shape=Mdiamond];
    done [label="Done", shape=Msquare];
    start -> done;
}
'''.strip()


def test_graph_settings_exposes_stylesheet_parse_lint_feedback_item_6_5_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    graph_settings_text = (
        repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx"
    ).read_text(encoding="utf-8")

    assert "const diagnostics = useStore((state) => state.diagnostics)" in graph_settings_text
    assert "const stylesheetDiagnostics = diagnostics.filter((diag) => diag.rule_id === 'stylesheet_syntax')" in graph_settings_text
    assert "const showStylesheetFeedback = hasStylesheetValue || stylesheetDiagnostics.length > 0" in graph_settings_text
    assert 'data-testid="graph-model-stylesheet-selector-guidance"' in graph_settings_text
    assert 'data-testid="graph-model-stylesheet-diagnostics"' in graph_settings_text
    assert "Stylesheet parse and selector lint checks passed in preview." in graph_settings_text


def test_preview_exposes_stylesheet_syntax_diagnostics_item_6_5_02() -> None:
    payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=INVALID_STYLESHEET_FLOW)))
    diagnostics = payload["diagnostics"]

    stylesheet_diags = [diag for diag in diagnostics if diag["rule_id"] == "stylesheet_syntax"]
    assert stylesheet_diags, "invalid stylesheet should surface stylesheet_syntax diagnostics"
    assert any(diag["severity"] == "error" for diag in stylesheet_diags)


def test_preview_exposes_stylesheet_syntax_diagnostics_for_whitespace_stylesheet_item_6_5_02() -> None:
    payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=WHITESPACE_STYLESHEET_FLOW)))
    diagnostics = payload["diagnostics"]

    stylesheet_diags = [diag for diag in diagnostics if diag["rule_id"] == "stylesheet_syntax"]
    assert stylesheet_diags, "whitespace stylesheet should surface stylesheet_syntax diagnostics"
    assert any(diag["severity"] == "error" for diag in stylesheet_diags)


def test_checklist_marks_item_6_5_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.5-02]" in checklist_text
