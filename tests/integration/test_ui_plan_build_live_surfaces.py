from pathlib import Path


def test_planning_and_build_workflows_share_live_status_log_artifact_surfaces_item_8_5_06() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(
        encoding="utf-8"
    )
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    terminal_text = (repo_root / "frontend" / "src" / "components" / "Terminal.tsx").read_text(encoding="utf-8")

    planning_launch_snippets = [
        "setSelectedRunId(runData.pipeline_id)",
        "setViewMode('execution')",
    ]
    for snippet in planning_launch_snippets:
        assert snippet in projects_panel_text, f"missing planning workflow launch snippet: {snippet}"

    build_launch_snippets = [
        "if (typeof runData?.pipeline_id === 'string') {",
        "setSelectedRunId(runData.pipeline_id)",
        "setViewMode('execution')",
    ]
    for snippet in build_launch_snippets:
        assert snippet in navbar_text, f"missing build workflow launch snippet: {snippet}"

    live_surface_snippets = [
        "data-testid=\"execution-footer-workflow-status\"",
        "data-testid=\"execution-footer-log-count\"",
        "data-testid=\"execution-footer-workflow-artifacts\"",
        "data-testid=\"execution-footer-workflow-artifact-status\"",
        "data-testid=\"execution-footer-workflow-artifact-link\"",
        "const selectedRunId = useStore((state) => state.selectedRunId)",
        "const runtimeStatus = useStore((state) => state.runtimeStatus)",
        "const logs = useStore((state) => state.logs)",
        "fetch(`/pipelines/${encodeURIComponent(selectedRunId)}/graph`",
    ]
    for snippet in live_surface_snippets:
        assert snippet in terminal_text, f"missing live status/log/artifact surface snippet: {snippet}"
