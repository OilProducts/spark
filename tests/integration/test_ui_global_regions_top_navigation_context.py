from pathlib import Path


def test_top_navigation_exposes_active_project_flow_and_run_context_item_4_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_path = repo_root / "frontend" / "src" / "components" / "Navbar.tsx"
    navbar_text = navbar_path.read_text(encoding="utf-8")

    required_snippets = [
        'data-testid="top-nav-active-project"',
        'data-testid="top-nav-active-flow"',
        'data-testid="top-nav-run-context"',
        'className="flex items-center gap-2 text-xs text-muted-foreground"',
        "activeProjectPath",
        "activeFlow",
        "runtimeStatus",
        "Execute",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing top navigation context snippet: {snippet}"


def test_route_state_persists_active_project_identity_for_top_nav_context_item_4_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_path = repo_root / "frontend" / "src" / "store.ts"
    store_text = store_path.read_text(encoding="utf-8")

    required_snippets = [
        "activeProjectPath: string | null",
        "setActiveProjectPath: (projectPath: string | null) => void",
        "activeProjectPath: typeof parsed.activeProjectPath === \"string\" ? parsed.activeProjectPath : null,",
        "activeProjectPath: restoredRouteState.activeProjectPath,",
        "setActiveProjectPath: (projectPath) =>",
    ]

    for snippet in required_snippets:
        assert snippet in store_text, f"missing active-project route persistence snippet: {snippet}"


def test_checklist_marks_item_4_1_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [4.1-01]" in checklist_text
