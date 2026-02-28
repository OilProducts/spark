from pathlib import Path


def test_parity_risk_report_exists_with_required_failure_mode_coverage() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    report_path = repo_root / "ui-parity-risk-report.md"

    assert report_path.exists(), "missing parity-risk report for checklist item 1.1-02"

    report_text = report_path.read_text(encoding="utf-8")

    required_snippets = [
        "Checklist item: [1.1-02]",
        "Behavior-Loss Failure Modes",
        "Hidden-Config Failure Modes",
        "stack.child_dotfile",
        "tool_hooks.pre",
        "manager.actions",
        "human.default_choice",
        "subgraph",
        "node[...] defaults",
        "edge[...] defaults",
        "Severity",
        "Mitigation direction",
    ]
    for snippet in required_snippets:
        assert snippet in report_text, f"missing required parity-risk coverage: {snippet}"


def test_checklist_marks_item_1_1_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [1.1-02]" in checklist_text
