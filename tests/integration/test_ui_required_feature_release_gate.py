from __future__ import annotations

from pathlib import Path

import pytest

from attractor.ui_release_gate import (
    extract_required_ui_feature_rows,
    enforce_required_ui_feature_release_gate,
    main,
    run_required_ui_feature_release_gate,
)


def test_ui_release_gate_fails_when_any_required_feature_is_unavailable() -> None:
    checklist_text = """
## Appendix A: Attribute Coverage Matrix
- [x] [A1-01] Implement + test `goal`.
- [ ] [A1-02] Implement + test `label`.

## Appendix B: Construct Coverage Matrix
- [x] [B-01] Implement + test directed graph authoring with valid node IDs.
"""

    rows = extract_required_ui_feature_rows(checklist_text)

    with pytest.raises(RuntimeError, match=r"A1-02"):
        enforce_required_ui_feature_release_gate(rows)


def test_ui_release_gate_allows_release_when_all_required_features_are_available() -> None:
    checklist_text = """
## Appendix A: Attribute Coverage Matrix
- [x] [A1-01] Implement + test `goal`.
- [x] [A2-01] Implement + test `label`.

## Appendix B: Construct Coverage Matrix
- [x] [B-01] Implement + test directed graph authoring with valid node IDs.
"""

    rows = extract_required_ui_feature_rows(checklist_text)

    enforce_required_ui_feature_release_gate(rows)


def test_ui_release_gate_fails_when_no_required_feature_rows_are_present() -> None:
    with pytest.raises(RuntimeError, match=r"no required checklist items"):
        enforce_required_ui_feature_release_gate(())


def test_ui_release_gate_runner_fails_for_checklist_file_with_unavailable_features(tmp_path) -> None:
    checklist_path = tmp_path / "ui-implementation-checklist.md"
    checklist_path.write_text(
        """
## Appendix A: Attribute Coverage Matrix
- [ ] [A1-01] Implement + test `goal`.
""",
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match=r"A1-01"):
        run_required_ui_feature_release_gate(checklist_path)


def test_ui_release_gate_cli_entrypoint_succeeds_for_checked_checklist(tmp_path) -> None:
    checklist_path = tmp_path / "ui-implementation-checklist.md"
    checklist_path.write_text(
        """
## Appendix A: Attribute Coverage Matrix
- [x] [A1-01] Implement + test `goal`.
## Appendix B: Construct Coverage Matrix
- [x] [B-01] Implement + test directed graph authoring with valid node IDs.
""",
        encoding="utf-8",
    )

    assert main(["--checklist", str(checklist_path)]) == 0


def test_checklist_marks_item_1_2_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [1.2-03]" in checklist_text
