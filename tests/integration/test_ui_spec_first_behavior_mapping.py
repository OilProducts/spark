from pathlib import Path


def test_spec_first_behavior_mapping_doc_exists_with_required_control_coverage() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    doc_path = repo_root / "ui-spec-first-behavior-map.md"

    assert doc_path.exists(), "missing spec-first behavior mapping doc for checklist item 2-01"

    doc_text = doc_path.read_text(encoding="utf-8")
    required_snippets = [
        "Checklist item: [2-01]",
        "ui-spec.md",
        "attractor-spec.md",
        "Control-to-Spec Behavior Map",
        "Top navigation mode switch (Editor/Execution/Settings/Runs)",
        "Execute button",
        "Add Node button",
        "Flow create/delete/select controls",
        "Graph settings drawer",
        "Apply To Nodes button",
        "Reset From Global button",
        "Node inspector fields",
        "Node quick-edit controls",
        "Edge inspector fields",
        "Validation panel entries",
        "Canvas controls (pan/zoom/fit/minimap)",
        "Run history refresh/open/cancel actions",
        "Execution footer cancel control",
        "Terminal clear action",
        "Spec references",
    ]

    for snippet in required_snippets:
        assert snippet in doc_text, f"missing required spec-first mapping coverage: {snippet}"

    # Require a broad mapping table so the checklist item does not pass with a minimal subset.
    map_lines = [line for line in doc_text.splitlines() if line.startswith("| ") and " | " in line]
    assert len(map_lines) >= 18, "control mapping is too narrow for item 2-01"



def test_checklist_marks_item_2_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [2-01]" in checklist_text
