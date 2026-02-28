from pathlib import Path


def test_store_enforces_active_project_for_editor_and_execution_item_4_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    required_snippets = [
        "const modeRequiresActiveProject = (mode: ViewMode) => mode === 'editor' || mode === 'execution'",
        "const resolveViewModeForProjectScope = (mode: ViewMode, activeProjectPath: string | null): ViewMode => {",
        "return modeRequiresActiveProject(mode) && !activeProjectPath ? 'projects' : mode",
        "const nextViewMode = resolveViewModeForProjectScope(mode, state.activeProjectPath)",
    ]

    for snippet in required_snippets:
        assert snippet in store_text, f"missing active-project mode enforcement snippet: {snippet}"


def test_default_route_state_starts_in_projects_without_active_project_item_4_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    assert "const DEFAULT_ROUTE_STATE: RouteState = {" in store_text
    assert "viewMode: 'projects'," in store_text
    assert "activeProjectPath: null," in store_text


def test_execute_action_requires_active_project_item_4_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "if (!activeProjectPath || !activeFlow || hasValidationErrors) return",
        "disabled={!activeProjectPath || !activeFlow || hasValidationErrors}",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing execute active-project guard snippet: {snippet}"


def test_checklist_marks_item_4_2_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [4.2-03]" in checklist_text
