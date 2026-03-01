from __future__ import annotations

from pathlib import Path


def test_start_and_cancel_controls_are_present_for_supported_backend_behavior_item_8_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    execution_controls_text = (repo_root / "frontend" / "src" / "components" / "ExecutionControls.tsx").read_text(
        encoding="utf-8"
    )
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    # Start control is wired to the supported /pipelines run-start contract.
    assert 'data-testid="execute-button"' in navbar_text
    assert "onClick={runPipeline}" in navbar_text
    assert "const runPipeline = async () => {" in navbar_text
    assert "const runRes = await fetch('/pipelines', {" in navbar_text
    assert "setViewMode('execution')" in navbar_text

    # Cancel control is wired to supported backend cancellation endpoints.
    assert 'data-testid="execution-footer-controls"' in execution_controls_text
    assert "const canCancel = runtimeStatus === 'running' && Boolean(selectedRunId)" in execution_controls_text
    assert "const response = await fetch(`/pipelines/${encodeURIComponent(selectedRunId)}/cancel`, { method: 'POST' })" in execution_controls_text
    assert "window.confirm('Cancel this run? It will stop after the active node finishes.')" in execution_controls_text

    # Run-history view also exposes cancel where supported.
    assert "const canCancel = run.status === 'running'" in runs_panel_text
    assert "const response = await fetch(`/pipelines/${encodeURIComponent(runId)}/cancel`, { method: 'POST' })" in runs_panel_text

    assert "- [x] [8.2-01]" in checklist_text


def test_unsupported_runtime_controls_show_disabled_reason_text_item_8_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    execution_controls_text = (repo_root / "frontend" / "src" / "components" / "ExecutionControls.tsx").read_text(
        encoding="utf-8"
    )
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    # Pause/resume controls are shown as disabled when backend runtime capability is unavailable.
    assert "const UNSUPPORTED_CONTROL_REASON = 'Pause/Resume is unavailable: backend runtime control API does not expose pause/resume.'" in execution_controls_text
    assert "data-testid=\"execution-footer-pause-button\"" in execution_controls_text
    assert "data-testid=\"execution-footer-resume-button\"" in execution_controls_text
    assert "disabled={true}" in execution_controls_text

    # Unsupported control reason text is rendered visibly, not only as a hover tooltip.
    assert "data-testid=\"execution-footer-unsupported-controls-reason\"" in execution_controls_text
    assert "{UNSUPPORTED_CONTROL_REASON}" in execution_controls_text

    assert "- [x] [8.2-03]" in checklist_text


def test_runtime_controls_expose_enable_disable_state_transitions_item_8_2_04() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    execution_controls_text = (repo_root / "frontend" / "src" / "components" / "ExecutionControls.tsx").read_text(
        encoding="utf-8"
    )
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    # Cancel is enabled only while a run is actively running and the selected run-id is hydrated.
    assert "const canCancel = runtimeStatus === 'running' && Boolean(selectedRunId)" in execution_controls_text

    # Transition-state disabled reasons are explicit for cancel-requested and terminal canceled states.
    assert "const CANCEL_DISABLED_REASONS: Record<string, string> = {" in execution_controls_text
    assert "cancel_requested: 'Cancel already requested for this run.'" in execution_controls_text
    assert "abort_requested: 'Cancel already requested for this run.'" in execution_controls_text
    assert "canceled: 'This run is already canceled.'" in execution_controls_text
    assert "aborted: 'This run is already canceled.'" in execution_controls_text

    # Disabled title text must resolve through transition-state reason wiring.
    assert "const cancelDisabledReason = !selectedRunId" in execution_controls_text
    assert "title={canCancel ? undefined : cancelDisabledReason}" in execution_controls_text
    assert "data-testid=\"execution-footer-cancel-button\"" in execution_controls_text

    assert "- [x] [8.2-04]" in checklist_text
