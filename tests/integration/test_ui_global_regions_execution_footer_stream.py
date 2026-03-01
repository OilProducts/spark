from pathlib import Path


def test_execution_footer_and_stream_regions_remain_consistent_during_active_runs_item_4_1_04() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    app_text = (repo_root / "frontend" / "src" / "App.tsx").read_text(encoding="utf-8")
    terminal_text = (repo_root / "frontend" / "src" / "components" / "Terminal.tsx").read_text(encoding="utf-8")
    execution_controls_text = (repo_root / "frontend" / "src" / "components" / "ExecutionControls.tsx").read_text(encoding="utf-8")
    run_stream_text = (repo_root / "frontend" / "src" / "components" / "RunStream.tsx").read_text(encoding="utf-8")

    # Canvas-primary areas must retain a stable footer + stream structure.
    assert '<div data-testid="canvas-workspace-primary"' in app_text
    assert "<Terminal />" in app_text

    # Runtime output stream remains in a dedicated execution footer region.
    assert 'data-testid="execution-footer-stream"' in terminal_text
    assert 'data-testid="execution-footer-terminal-output"' in terminal_text
    assert 'data-testid="execution-footer-terminal-clear"' in terminal_text

    # Footer controls remain visible for active runs, even if run-id hydration lags briefly.
    assert "const runIsActive = ACTIVE_RUNTIME_STATUSES.has(runtimeStatus)" in execution_controls_text
    assert "const shouldShowFooter = viewMode === 'execution' && (runIsActive || Boolean(selectedRunId))" in execution_controls_text
    assert 'data-testid="execution-footer-controls"' in execution_controls_text
    assert 'data-testid="execution-footer-run-status"' in execution_controls_text

    # Global runtime stream indicator is stable and test-addressable.
    assert 'data-testid="execution-runtime-stream-indicator"' in run_stream_text


def test_checklist_marks_item_4_1_04_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [4.1-04]" in checklist_text


def test_execution_footer_visibility_tracks_active_runtime_states_item_8_4_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    execution_controls_text = (repo_root / "frontend" / "src" / "components" / "ExecutionControls.tsx").read_text(
        encoding="utf-8"
    )
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    # Active-run state handling is explicit and drives execution-footer visibility.
    assert "const ACTIVE_RUNTIME_STATUSES = new Set<RuntimeStatus>([" in execution_controls_text
    assert "'running'" in execution_controls_text
    assert "'cancel_requested'" in execution_controls_text
    assert "'abort_requested'" in execution_controls_text
    assert "const runIsActive = ACTIVE_RUNTIME_STATUSES.has(runtimeStatus)" in execution_controls_text
    assert "const shouldShowFooter = viewMode === 'execution' && (runIsActive || Boolean(selectedRunId))" in execution_controls_text

    assert "- [x] [8.4-01]" in checklist_text
