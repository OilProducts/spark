from pathlib import Path


def test_navbar_has_persistent_active_project_indicator_item_4_3_04() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    app_text = (repo_root / "frontend" / "src" / "App.tsx").read_text(encoding="utf-8")
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    required_navbar_snippets = [
        'data-testid="top-nav-active-project"',
        'const projectLabel = activeProjectPath || "No active project"',
        '<span className="font-medium text-foreground">Project:</span> {projectLabel}',
    ]

    for snippet in required_navbar_snippets:
        assert snippet in navbar_text, f"missing active-project indicator snippet: {snippet}"

    assert "<Navbar />" in app_text

    required_store_snippets = [
        "const restoredRouteState = loadRouteState()",
        "activeProjectPath: restoredRouteState.activeProjectPath,",
        "setActiveProjectPath: (projectPath) =>",
        "saveRouteState({",
        "activeProjectPath: projectPath,",
        "activeProjectPath: typeof parsed.activeProjectPath === \"string\" ? parsed.activeProjectPath : null,",
    ]

    for snippet in required_store_snippets:
        assert snippet in store_text, f"missing active-project persistence snippet: {snippet}"


def test_checklist_marks_item_4_3_04_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [4.3-04]" in checklist_text
