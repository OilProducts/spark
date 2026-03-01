from __future__ import annotations

from pathlib import Path


def test_inspector_field_level_diagnostic_mapping_wired_item_7_1_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    mapping_text = (repo_root / "frontend" / "src" / "lib" / "inspectorFieldDiagnostics.ts").read_text(
        encoding="utf-8"
    )
    sidebar_text = (repo_root / "frontend" / "src" / "components" / "Sidebar.tsx").read_text(
        encoding="utf-8"
    )
    graph_settings_text = (
        repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx"
    ).read_text(encoding="utf-8")

    assert "resolveNodeFieldDiagnostics" in mapping_text
    assert "resolveEdgeFieldDiagnostics" in mapping_text
    assert "resolveGraphFieldDiagnostics" in mapping_text
    assert "replaceAll('fallback_retry_target', '')" in mapping_text

    assert "resolveNodeFieldDiagnostics" in sidebar_text
    assert "resolveEdgeFieldDiagnostics" in sidebar_text
    assert "node-field-diagnostics-prompt" in sidebar_text
    assert "node-field-diagnostics-type" in sidebar_text
    assert "node-field-diagnostics-goal_gate" in sidebar_text
    assert "node-field-diagnostics-retry_target" in sidebar_text
    assert "node-field-diagnostics-fallback_retry_target" in sidebar_text
    assert "node-field-diagnostics-fidelity" in sidebar_text
    assert "edge-field-diagnostics-condition" in sidebar_text
    assert "edge-field-diagnostics-fidelity" in sidebar_text

    assert "resolveGraphFieldDiagnostics" in graph_settings_text
    assert "graph-field-diagnostics-default_fidelity" in graph_settings_text
    assert "graph-field-diagnostics-retry_target" in graph_settings_text
    assert "graph-field-diagnostics-fallback_retry_target" in graph_settings_text


def test_ui_smoke_covers_inspector_field_level_mapping_item_7_1_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "inspector field-level diagnostics map to matching fields for item 7.1-03" in ui_smoke_text
    assert "15-inspector-field-level-diagnostics.png" in ui_smoke_text
    assert "node-field-diagnostics-prompt" in ui_smoke_text
    assert "edge-field-diagnostics-condition" in ui_smoke_text


def test_checklist_marks_item_7_1_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [7.1-03]" in checklist_text
