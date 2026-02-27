from __future__ import annotations

import json

import pytest

from attractor.parity_matrix import (
    CROSS_FEATURE_PARITY_MATRIX_ROWS,
    enforce_cross_feature_parity_release_gate,
    run_cross_feature_parity_matrix,
)


def test_cross_feature_parity_matrix_executes_and_persists_report(tmp_path) -> None:
    report_path = tmp_path / "parity-matrix-report.json"

    report = run_cross_feature_parity_matrix(report_path)

    assert report_path.exists()
    persisted = json.loads(report_path.read_text(encoding="utf-8"))
    assert persisted == report
    assert report["summary"]["total"] == len(CROSS_FEATURE_PARITY_MATRIX_ROWS)
    assert report["summary"]["passed"] + report["summary"]["failed"] == len(CROSS_FEATURE_PARITY_MATRIX_ROWS)
    assert [row["name"] for row in report["rows"]] == CROSS_FEATURE_PARITY_MATRIX_ROWS


def test_release_gate_fails_when_any_matrix_row_is_unchecked() -> None:
    report = {
        "rows": [
            {"name": CROSS_FEATURE_PARITY_MATRIX_ROWS[0], "pass": True},
            {"name": CROSS_FEATURE_PARITY_MATRIX_ROWS[1], "pass": False},
        ]
    }

    with pytest.raises(RuntimeError, match="unchecked"):
        enforce_cross_feature_parity_release_gate(report)


def test_release_gate_allows_all_checked_rows() -> None:
    report = {"rows": [{"name": name, "pass": True} for name in CROSS_FEATURE_PARITY_MATRIX_ROWS]}

    enforce_cross_feature_parity_release_gate(report)
