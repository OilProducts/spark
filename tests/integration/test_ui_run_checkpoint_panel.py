from __future__ import annotations

from pathlib import Path


def test_runs_panel_adds_checkpoint_viewer_backed_by_checkpoint_endpoint_item_9_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/checkpoint`)",
        "data-testid=\"run-checkpoint-panel\"",
        "data-testid=\"run-checkpoint-refresh-button\"",
        "data-testid=\"run-checkpoint-payload\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing checkpoint viewer snippet: {snippet}"


def test_ui_smoke_includes_checkpoint_viewer_visual_qa_item_9_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run checkpoint viewer fetches checkpoint payload for item 9.2-01" in ui_smoke_text
    assert "08d-runs-panel-checkpoint-viewer.png" in ui_smoke_text


def test_runs_panel_renders_checkpoint_progress_fields_item_9_2_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "data-testid=\"run-checkpoint-current-node\"",
        "data-testid=\"run-checkpoint-completed-nodes\"",
        "data-testid=\"run-checkpoint-retry-counters\"",
        "data-testid=\"run-checkpoint-payload\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing checkpoint progress field snippet: {snippet}"
