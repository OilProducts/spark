from __future__ import annotations

from pathlib import Path


def test_runs_panel_adds_context_viewer_backed_by_context_endpoint_item_9_3_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/context`)",
        "data-testid=\"run-context-panel\"",
        "data-testid=\"run-context-search-input\"",
        "data-testid=\"run-context-table\"",
        "data-testid=\"run-context-row-type\"",
        "data-testid=\"run-context-row-value\"",
        "data-testid=\"run-context-empty\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing context viewer snippet: {snippet}"


def test_ui_smoke_includes_context_viewer_visual_qa_item_9_3_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run context viewer supports searchable key/value inspection for item 9.3-01" in ui_smoke_text
    assert "08f-runs-panel-context-viewer.png" in ui_smoke_text
    assert "run-context-row-type" in ui_smoke_text
    assert "run-context-row-value" in ui_smoke_text
    assert "No context entries match the current search." in ui_smoke_text
