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


def test_execute_warning_only_state_shows_explicit_warning_banner_item_7_2_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    assert "const diagnostics = useStore((state) => state.diagnostics)" in navbar_text
    assert "const hasValidationWarnings = diagnostics.some((diag) => diag.severity === 'warning')" in navbar_text
    assert "const showValidationWarningBanner = hasValidationWarnings && !hasValidationErrors" in navbar_text
    assert 'data-testid="execute-warning-banner"' in navbar_text
    assert "Warnings present; run allowed." in navbar_text


def test_ui_smoke_covers_warning_only_execute_state_item_7_2_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "warning-only diagnostics still allow execute with explicit banner for item 7.2-02" in ui_smoke_text
    assert "16-warning-only-execute-banner.png" in ui_smoke_text
    assert "execute-warning-banner" in ui_smoke_text


def test_ui_smoke_covers_diagnostic_transition_blocking_unblocking_item_7_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "diagnostics transitions toggle execute blocking and warning state for item 7.2-03" in ui_smoke_text
    assert "17-diagnostic-transition-execute-state.png" in ui_smoke_text
    assert "Fix validation errors before running." in ui_smoke_text
    assert "execute-warning-banner" in ui_smoke_text


def test_checklist_marks_item_7_2_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.2-01]" in checklist_text


def test_checklist_marks_item_7_2_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.2-02]" in checklist_text


def test_checklist_marks_item_7_2_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.2-03]" in checklist_text
