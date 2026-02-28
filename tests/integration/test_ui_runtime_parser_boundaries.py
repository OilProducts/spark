from pathlib import Path


def test_runtime_parser_boundaries_doc_exists_with_required_coverage() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    doc_path = repo_root / "ui-runtime-parser-boundaries.md"

    assert doc_path.exists(), "missing runtime/parser boundaries doc for checklist item 1.3-01"

    doc_text = doc_path.read_text(encoding="utf-8")
    required_snippets = [
        "Checklist item: [1.3-01]",
        "ui-spec.md",
        "attractor-spec.md",
        "Non-Goals",
        "Replacing the DOT runtime parser or executor",
        "UI-owned responsibilities",
        "Runtime/parser-owned responsibilities",
        "Boundary Rules",
        "do not reinterpret execution semantics",
        "no runtime parser changes",
    ]
    for snippet in required_snippets:
        assert snippet in doc_text, f"missing required runtime/parser boundary coverage: {snippet}"


def test_checklist_marks_item_1_3_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [1.3-01]" in checklist_text
