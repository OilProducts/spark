from pathlib import Path


def test_projects_panel_launches_plan_generation_only_from_approved_spec_state_item_8_5_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(
        encoding="utf-8"
    )
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    store_snippets = [
        "specStatus: 'draft' | 'approved'",
        "setSpecStatus: (status: 'draft' | 'approved') => void",
        "specStatus: 'draft',",
    ]
    for snippet in store_snippets:
        assert snippet in store_text, f"missing project-scoped spec approval state snippet: {snippet}"

    required_snippets = [
        "data-testid=\"project-plan-generation-surface\"",
        "const specIsApprovedForPlanning = activeProjectScope?.specStatus === 'approved'",
        "data-testid=\"project-spec-approve-for-plan-button\"",
        "Approve spec for planning",
        "data-testid=\"project-plan-generation-launch-button\"",
        "Launch plan-generation workflow",
        "if (!activeProjectPath || !activeProjectScope?.specId || !specIsApprovedForPlanning) {",
        "const flowRes = await fetch(`/api/flows/${encodeURIComponent(activeFlow)}`)",
        "const runRes = await fetch('/pipelines', {",
        "setSelectedRunId(runData.pipeline_id)",
        "setViewMode('execution')",
    ]
    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing plan-generation launch workflow snippet: {snippet}"


def test_checklist_marks_item_8_5_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [8.5-01]" in checklist_text
