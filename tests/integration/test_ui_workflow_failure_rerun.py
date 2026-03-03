from pathlib import Path


def test_projects_panel_exposes_plan_failure_diagnostics_with_rerun_affordance_item_8_5_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(
        encoding="utf-8"
    )

    required_snippets = [
        "const [lastPlanGenerationFailure, setLastPlanGenerationFailure] = useState<WorkflowFailureDiagnostics | null>(null)",
        "setLastPlanGenerationFailure({",
        "data-testid=\"project-plan-failure-diagnostics\"",
        "data-testid=\"project-plan-failure-message\"",
        "data-testid=\"project-plan-generation-rerun-button\"",
        "Retry plan-generation workflow",
    ]

    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing plan workflow failure diagnostics/rerun snippet: {snippet}"


def test_navbar_exposes_build_failure_diagnostics_with_rerun_affordance_item_8_5_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const [lastBuildWorkflowFailure, setLastBuildWorkflowFailure] = useState<WorkflowFailureDiagnostics | null>(null)",
        "setLastBuildWorkflowFailure({",
        "data-testid=\"build-workflow-failure-diagnostics\"",
        "data-testid=\"build-workflow-failure-message\"",
        "data-testid=\"build-workflow-rerun-button\"",
        "Rerun build workflow",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing build workflow failure diagnostics/rerun snippet: {snippet}"
