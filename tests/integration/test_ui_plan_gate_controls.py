from pathlib import Path


def test_projects_panel_exposes_plan_gate_controls_with_explicit_status_transitions_item_8_5_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(
        encoding="utf-8"
    )
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    store_snippets = [
        "type PlanStatus = 'draft' | 'approved' | 'rejected' | 'revision-requested'",
        "planStatus: PlanStatus",
        "setPlanStatus: (status: PlanStatus) => void",
        "planStatus: 'draft',",
    ]
    for snippet in store_snippets:
        assert snippet in store_text, f"missing plan status state snippet: {snippet}"

    required_projects_panel_snippets = [
        "const PLAN_STATUS_TRANSITIONS: Record<PlanStatus, PlanStatus[]> = {",
        "const canTransitionPlanStatus = (from: PlanStatus, to: PlanStatus) =>",
        "const onPlanGateTransition = (nextStatus: PlanStatus) => {",
        "data-testid=\"project-plan-gate-surface\"",
        "data-testid=\"project-plan-approve-button\"",
        "data-testid=\"project-plan-reject-button\"",
        "data-testid=\"project-plan-request-revision-button\"",
        "Plan status:",
        "if (!activeProjectPath || !activeProjectScope?.planId) {",
        "if (!canTransitionPlanStatus(activeProjectScope.planStatus, nextStatus)) {",
    ]
    for snippet in required_projects_panel_snippets:
        assert snippet in projects_panel_text, f"missing plan gate control snippet: {snippet}"
