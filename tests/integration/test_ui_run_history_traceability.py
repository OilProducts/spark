from pathlib import Path


def test_runs_panel_run_history_rows_include_project_and_git_metadata_item_9_6_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "data-testid=\"run-history-row-project-path\"",
        "data-testid=\"run-history-row-git-branch\"",
        "data-testid=\"run-history-row-git-commit\"",
        "Project:",
        "Branch:",
        "Commit:",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run-history traceability snippet: {snippet}"


def test_ui_smoke_includes_run_history_traceability_visual_qa_item_9_6_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run history rows include project identity and git metadata for item 9.6-02" in ui_smoke_text
    assert "08p-runs-panel-run-history-traceability.png" in ui_smoke_text


def test_runs_panel_run_history_rows_link_spec_and_plan_artifacts_item_9_6_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "data-testid=\"run-history-row-spec-artifact-link\"",
        "data-testid=\"run-history-row-plan-artifact-link\"",
        "Spec artifact:",
        "Plan artifact:",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run-history artifact linkage snippet: {snippet}"


def test_ui_smoke_includes_run_history_spec_plan_artifact_link_visual_qa_item_9_6_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run history rows link associated spec and plan artifacts when available for item 9.6-03" in ui_smoke_text
    assert "08q-runs-panel-run-history-spec-plan-links.png" in ui_smoke_text
