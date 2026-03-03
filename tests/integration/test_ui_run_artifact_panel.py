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


def test_runs_panel_adds_graphviz_render_viewer_for_item_9_5_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/graph`)",
        "data-testid=\"run-graphviz-panel\"",
        "data-testid=\"run-graphviz-refresh-button\"",
        "data-testid=\"run-graphviz-viewer\"",
        "data-testid=\"run-graphviz-viewer-image\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run graphviz viewer snippet: {snippet}"


def test_ui_smoke_includes_graphviz_render_viewer_visual_qa_item_9_5_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run graphviz viewer renders /pipelines/{id}/graph output for item 9.5-02" in ui_smoke_text
    assert "08n-runs-panel-graphviz-viewer.png" in ui_smoke_text
    assert "run-graphviz-viewer-image" in ui_smoke_text


def test_runs_panel_adds_graceful_missing_artifact_and_partial_run_states_item_9_5_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "data-testid=\"run-artifact-partial-run-note\"",
        "Artifact preview unavailable because the file was not found for this run.",
        "This run may be partial or artifacts may have been pruned.",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing graceful artifact handling snippet: {snippet}"


def test_ui_smoke_includes_missing_artifact_and_partial_run_visual_qa_item_9_5_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run artifact browser handles missing files and partial run states for item 9.5-03" in ui_smoke_text
    assert "08o-runs-panel-artifact-missing-partial.png" in ui_smoke_text
    assert "run-artifact-partial-run-note" in ui_smoke_text
