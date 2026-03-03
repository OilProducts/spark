from pathlib import Path


def test_runs_panel_renders_run_summary_fields_item_9_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "data-testid=\"run-summary-panel\"",
        "data-testid=\"run-summary-status\"",
        "data-testid=\"run-summary-result\"",
        "data-testid=\"run-summary-started-at\"",
        "data-testid=\"run-summary-ended-at\"",
        "data-testid=\"run-summary-duration\"",
        "data-testid=\"run-summary-model\"",
        "data-testid=\"run-summary-working-directory\"",
        "data-testid=\"run-summary-project-path\"",
        "data-testid=\"run-summary-git-branch\"",
        "data-testid=\"run-summary-git-commit\"",
        "data-testid=\"run-summary-last-error\"",
        "data-testid=\"run-summary-token-usage\"",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run summary panel snippet: {snippet}"

