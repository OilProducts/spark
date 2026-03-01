from __future__ import annotations

from pathlib import Path


def test_execute_is_blocked_only_by_error_level_diagnostics_item_7_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    assert "hasValidationErrors: diagnostics.some((diag) => diag.severity === 'error')" in store_text
    assert "if (!activeProjectPath || !activeFlow || hasValidationErrors) return" in navbar_text
    assert "disabled={!activeProjectPath || !activeFlow || hasValidationErrors}" in navbar_text
    assert "? 'Fix validation errors before running.'" in navbar_text


def test_checklist_marks_item_7_2_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.2-01]" in checklist_text
