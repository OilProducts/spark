from pathlib import Path


def test_projects_panel_exposes_project_scoped_conversation_spec_plan_entry_points_item_4_2_06() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        'data-testid="project-scope-entrypoints"',
        'data-testid="project-conversation-entrypoint"',
        'data-testid="project-spec-entrypoint"',
        'data-testid="project-plan-entrypoint"',
        "Conversation",
        "Spec",
        "Plan",
    ]

    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing Projects workspace entry-point snippet: {snippet}"


def test_projects_panel_scopes_entry_points_to_active_project_item_4_2_06() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null",
        "Select an active project to access conversation, spec, and plan entry points.",
    ]

    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing active-project scoped entry-point behavior snippet: {snippet}"
