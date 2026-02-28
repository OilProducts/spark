from pathlib import Path


def test_store_rejects_duplicate_project_paths_for_create_and_update_item_4_3_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    required_snippets = [
        "registerProject: (directoryPath: string) => ProjectRegistrationResult",
        "updateProjectPath: (currentDirectoryPath: string, nextDirectoryPath: string) => ProjectRegistrationResult",
        "const duplicate = Boolean(state.projectRegistry[normalizedPath])",
        "const duplicate = normalizedNextPath !== normalizedCurrentPath && Boolean(state.projectRegistry[normalizedNextPath])",
        "error: `Project already registered: ${normalizedPath}`",
        "error: `Project already registered: ${normalizedNextPath}`",
    ]

    for snippet in required_snippets:
        assert snippet in store_text, f"missing duplicate-path prevention snippet: {snippet}"


def test_projects_panel_exposes_project_path_update_controls_item_4_3_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        'data-testid="project-edit-button"',
        'data-testid="project-edit-input"',
        'data-testid="project-edit-save-button"',
        'data-testid="project-edit-cancel-button"',
        "updateProjectPath(project.directoryPath, editingDirectoryPathInput)",
    ]

    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing project update control snippet: {snippet}"


def test_checklist_marks_item_4_3_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [4.3-02]" in checklist_text
