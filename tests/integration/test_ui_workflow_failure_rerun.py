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
        "data-testid=\"project-plan-generation-rerun-disabled-reason\"",
        "Fix launch prerequisites to enable rerun.",
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
        "data-testid=\"build-workflow-rerun-disabled-reason\"",
        "Resolve launch blockers to rerun build.",
        "Rerun build workflow",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing build workflow failure diagnostics/rerun snippet: {snippet}"


def test_ui_smoke_covers_planning_and_build_failure_rerun_states_item_8_5_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    required_snippets = [
        "planning/build failures show diagnostics and rerun affordances for item 8.5-05",
        "project-plan-failure-diagnostics",
        "project-plan-generation-rerun-button",
        "project-plan-generation-rerun-disabled-reason",
        "build-workflow-failure-diagnostics",
        "build-workflow-rerun-button",
        "build-workflow-rerun-disabled-reason",
        "20a-plan-failure-rerun-enabled.png",
        "20b-plan-failure-rerun-disabled.png",
        "20c-build-failure-rerun-enabled.png",
        "20d-build-failure-rerun-disabled.png",
    ]

    for snippet in required_snippets:
        assert snippet in ui_smoke_text, f"missing item 8.5-05 UI smoke snippet: {snippet}"
