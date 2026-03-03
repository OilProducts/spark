from __future__ import annotations

from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_test_passed
from tests.contracts.frontend._support.preview_api import preview_pipeline


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
    assert_frontend_behavior_test_passed("renders graph settings feedback for stylesheet diagnostics and tool hook warnings")


def test_preview_exposes_stylesheet_syntax_diagnostics_item_6_5_02() -> None:
    payload = preview_pipeline(INVALID_STYLESHEET_FLOW)
    diagnostics = payload["diagnostics"]

    stylesheet_diags = [diag for diag in diagnostics if diag["rule_id"] == "stylesheet_syntax"]
    assert stylesheet_diags, "invalid stylesheet should surface stylesheet_syntax diagnostics"
    assert any(diag["severity"] == "error" for diag in stylesheet_diags)


def test_preview_exposes_stylesheet_syntax_diagnostics_for_whitespace_stylesheet_item_6_5_02() -> None:
    payload = preview_pipeline(WHITESPACE_STYLESHEET_FLOW)
    diagnostics = payload["diagnostics"]

    stylesheet_diags = [diag for diag in diagnostics if diag["rule_id"] == "stylesheet_syntax"]
    assert stylesheet_diags, "whitespace stylesheet should surface stylesheet_syntax diagnostics"
    assert any(diag["severity"] == "error" for diag in stylesheet_diags)
