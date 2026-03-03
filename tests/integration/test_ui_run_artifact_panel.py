from __future__ import annotations

from pathlib import Path


def test_runs_panel_adds_artifact_listing_and_view_download_actions_item_9_5_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/artifacts`)",
        "data-testid=\"run-artifact-panel\"",
        "data-testid=\"run-artifact-refresh-button\"",
        "data-testid=\"run-artifact-table\"",
        "data-testid=\"run-artifact-row\"",
        "data-testid=\"run-artifact-view-button\"",
        "data-testid=\"run-artifact-download-link\"",
        "data-testid=\"run-artifact-viewer\"",
        "data-testid=\"run-artifact-viewer-payload\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run artifact browser snippet: {snippet}"


def test_ui_smoke_includes_run_artifact_browser_visual_qa_item_9_5_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run artifact browser lists run outputs and supports view/download for item 9.5-01" in ui_smoke_text
    assert "08m-runs-panel-artifact-browser.png" in ui_smoke_text
    assert "run-artifact-view-button" in ui_smoke_text
