import re
from pathlib import Path


def test_projects_panel_reject_action_discards_proposal_without_spec_mutation_item_5_5_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")

    assert 'data-testid="project-spec-edit-proposal-reject-button"' in projects_panel_text
    assert "Reject proposal" in projects_panel_text

    reject_handler_match = re.search(
        r"const onRejectSpecEditProposal = \(\) => \{(?P<body>.*?)\n    \}\n\n    return \(",
        projects_panel_text,
        flags=re.DOTALL,
    )
    assert reject_handler_match, "missing reject handler for spec edit proposals"

    reject_handler_body = reject_handler_match.group("body")
    assert "setSpecId" not in reject_handler_body, "reject action must not mutate spec artifact selection"
    assert "clearProjectSpecEditProposal(current, activeProjectPath)" in reject_handler_body, (
        "reject action must clear the active proposal preview"
    )


def test_checklist_marks_item_5_5_05_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [5.5-05]" in checklist_text
