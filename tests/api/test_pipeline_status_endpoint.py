from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import spark.app as product_app
from attractor.api.token_usage import TokenUsageBreakdown, TokenUsageBucket
from attractor.engine import Checkpoint, save_checkpoint
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_terminal_status as _wait_for_pipeline_terminal_status,
)


def _write_checkpoint(run_root: Path, current_node: str, completed_nodes: list[str]) -> None:
    save_checkpoint(
        run_root / "state.json",
        Checkpoint(
            current_node=current_node,
            completed_nodes=completed_nodes,
            context={},
            retry_counts={},
        ),
    )


def test_get_pipeline_returns_progress_for_active_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="plan", completed_nodes=["start"])

    response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["run_id"] == run_id
    assert payload["status"] == "running"
    assert "flow_name" in payload
    assert payload["working_directory"] == str(tmp_path / "work")
    assert payload["project_path"] == str((tmp_path / "work").resolve())
    assert payload["git_branch"] is None
    assert payload["git_commit"] is None
    assert payload["spec_id"] is None
    assert payload["plan_id"] is None
    assert payload["started_at"]
    assert payload["ended_at"] is None
    assert payload["token_usage"] is None
    assert payload["token_usage_breakdown"] is None
    assert payload["estimated_model_cost"] is None
    assert payload["completed_nodes"] == ["start"]
    assert payload["progress"] == {
        "current_node": "plan",
        "completed_count": 1,
    }


def test_get_pipeline_uses_checkpoint_progress_for_persisted_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    final_status = _wait_for_pipeline_terminal_status(attractor_api_client, run_id)
    assert final_status == "completed"

    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="done", completed_nodes=["start", "plan"])

    response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["run_id"] == run_id
    assert payload["status"] == "completed"
    assert payload["outcome"] == "success"
    assert "flow_name" in payload
    assert payload["working_directory"] == str(tmp_path / "work")
    assert payload["project_path"] == str((tmp_path / "work").resolve())
    assert payload["git_branch"] is None
    assert payload["git_commit"] is None
    assert payload["spec_id"] is None
    assert payload["plan_id"] is None
    assert payload["started_at"]
    assert payload["ended_at"]
    assert payload["token_usage"] is None
    assert payload["token_usage_breakdown"] is None
    assert payload["estimated_model_cost"] is None
    assert payload["completed_nodes"] == ["start", "plan"]
    assert payload["progress"] == {
        "current_node": "done",
        "completed_count": 2,
    }


def test_get_pipeline_preserves_persisted_metadata_while_overlaying_active_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    run_id = "run-active-detail"
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    server._record_run_start(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-detail",
        spec_id="spec-123",
        plan_id="plan-456",
        continued_from_run_id="run-parent",
        continued_from_node="Audit Milestone",
        continued_from_flow_mode="snapshot",
        continued_from_flow_name="implement-spec.dot",
    )
    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="review", completed_nodes=["start"])
    (run_root / "run.log").write_text("tokens used: 321\n", encoding="utf-8")
    server.ACTIVE_RUNS[run_id] = server.ActiveRun(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-detail",
        status="cancel_requested",
        last_error="waiting for graceful shutdown",
        completed_nodes=["start", "plan"],
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["run_id"] == run_id
    assert payload["status"] == "cancel_requested"
    assert payload["outcome"] is None
    assert payload["outcome_reason_code"] is None
    assert payload["outcome_reason_message"] is None
    assert payload["flow_name"] == "detail.dot"
    assert payload["working_directory"] == str(workdir)
    assert payload["project_path"] == str(workdir.resolve())
    assert payload["git_branch"] is None
    assert payload["git_commit"] is None
    assert payload["model"] == "gpt-detail"
    assert payload["spec_id"] == "spec-123"
    assert payload["plan_id"] == "plan-456"
    assert payload["continued_from_run_id"] == "run-parent"
    assert payload["continued_from_node"] == "Audit Milestone"
    assert payload["continued_from_flow_mode"] == "snapshot"
    assert payload["continued_from_flow_name"] == "implement-spec.dot"
    assert payload["last_error"] == "waiting for graceful shutdown"
    assert payload["completed_nodes"] == ["start", "plan"]
    assert payload["token_usage"] == 321
    assert payload["token_usage_breakdown"] is None
    assert payload["estimated_model_cost"] is None
    assert payload["progress"] == {
        "current_node": "review",
        "completed_count": 2,
    }
    server.ACTIVE_RUNS.pop(run_id, None)


def test_get_pipeline_returns_full_persisted_detail_for_completed_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    run_id = "run-completed-detail"
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    server._record_run_start(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-detail",
        spec_id="spec-123",
        plan_id="plan-456",
        continued_from_run_id="run-parent",
        continued_from_node="Audit Milestone",
        continued_from_flow_mode="flow_name",
        continued_from_flow_name="implement-spec.dot",
    )
    server._record_run_end(
        run_id=run_id,
        working_directory=str(workdir),
        status="completed",
        outcome="success",
        outcome_reason_code=None,
        outcome_reason_message=None,
    )
    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="done", completed_nodes=["start", "plan", "review"])
    (run_root / "run.log").write_text("tokens used: 987\n", encoding="utf-8")

    response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["run_id"] == run_id
    assert payload["status"] == "completed"
    assert payload["outcome"] == "success"
    assert payload["flow_name"] == "detail.dot"
    assert payload["working_directory"] == str(workdir)
    assert payload["project_path"] == str(workdir.resolve())
    assert payload["git_branch"] is None
    assert payload["git_commit"] is None
    assert payload["model"] == "gpt-detail"
    assert payload["spec_id"] == "spec-123"
    assert payload["plan_id"] == "plan-456"
    assert payload["continued_from_run_id"] == "run-parent"
    assert payload["continued_from_node"] == "Audit Milestone"
    assert payload["continued_from_flow_mode"] == "flow_name"
    assert payload["continued_from_flow_name"] == "implement-spec.dot"
    assert payload["completed_nodes"] == ["start", "plan", "review"]
    assert payload["token_usage"] == 987
    assert payload["token_usage_breakdown"] is None
    assert payload["estimated_model_cost"] is None
    assert payload["started_at"]
    assert payload["ended_at"]
    assert payload["progress"] == {
        "current_node": "done",
        "completed_count": 3,
    }


def test_get_pipeline_returns_structured_usage_and_estimated_cost(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    run_id = "run-structured-usage"
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    server._record_run_start(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-5.5",
    )
    server._record_run_usage(
        run_id,
        TokenUsageBreakdown(
            input_tokens=23,
            cached_input_tokens=3,
            output_tokens=13,
            total_tokens=36,
            by_model={
                "gpt-5.5": TokenUsageBucket(
                    input_tokens=15,
                    cached_input_tokens=3,
                    output_tokens=9,
                    total_tokens=24,
                ),
                "gpt-5.3-codex-spark": TokenUsageBucket(
                    input_tokens=8,
                    cached_input_tokens=0,
                    output_tokens=4,
                    total_tokens=12,
                ),
            },
        ),
    )
    server.ACTIVE_RUNS[run_id] = server.ActiveRun(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-5.5",
        status="running",
    )
    server._set_active_run_usage(
        run_id,
        TokenUsageBreakdown(
            input_tokens=23,
            cached_input_tokens=3,
            output_tokens=13,
            total_tokens=36,
            by_model={
                "gpt-5.5": TokenUsageBucket(
                    input_tokens=15,
                    cached_input_tokens=3,
                    output_tokens=9,
                    total_tokens=24,
                ),
                "gpt-5.3-codex-spark": TokenUsageBucket(
                    input_tokens=8,
                    cached_input_tokens=0,
                    output_tokens=4,
                    total_tokens=12,
                ),
            },
        ),
    )

    response = attractor_api_client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["token_usage"] == 36
    assert payload["token_usage_breakdown"] == {
        "input_tokens": 23,
        "cached_input_tokens": 3,
        "output_tokens": 13,
        "total_tokens": 36,
        "by_model": {
            "gpt-5.3-codex-spark": {
                "input_tokens": 8,
                "cached_input_tokens": 0,
                "output_tokens": 4,
                "total_tokens": 12,
            },
            "gpt-5.5": {
                "input_tokens": 15,
                "cached_input_tokens": 3,
                "output_tokens": 9,
                "total_tokens": 24,
            },
        },
    }
    assert payload["estimated_model_cost"]["currency"] == "USD"
    assert payload["estimated_model_cost"]["status"] == "partial_unpriced"
    assert payload["estimated_model_cost"]["amount"] == pytest.approx(0.000332, rel=0, abs=1e-9)
    assert payload["estimated_model_cost"]["unpriced_models"] == ["gpt-5.3-codex-spark"]
    assert payload["estimated_model_cost"]["by_model"]["gpt-5.3-codex-spark"] == {
        "currency": "USD",
        "amount": None,
        "status": "unpriced",
    }
    assert payload["estimated_model_cost"]["by_model"]["gpt-5.5"]["currency"] == "USD"
    assert payload["estimated_model_cost"]["by_model"]["gpt-5.5"]["status"] == "estimated"
    assert payload["estimated_model_cost"]["by_model"]["gpt-5.5"]["amount"] == pytest.approx(
        0.000332,
        rel=0,
        abs=1e-9,
    )
    server.ACTIVE_RUNS.pop(run_id, None)


def test_attractor_startup_reconciles_orphaned_running_run(tmp_path: Path) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

    run_id = "run-orphaned-running"
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    server._record_run_start(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-detail",
    )
    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="implement", completed_nodes=["start", "plan"])

    with TestClient(server.attractor_app) as client:
        response = client.get(f"/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "failed"
    assert payload["outcome"] is None
    assert payload["last_error"] == "Run was interrupted when the Attractor server stopped before completion."
    assert payload["ended_at"]

    persisted = server.pipeline_runs.read_run_meta(server.pipeline_runs.run_meta_path(server.get_runtime_paths, run_id))
    assert persisted is not None
    assert persisted.status == "failed"
    assert persisted.ended_at
    assert "Reconciled orphaned active run after server restart" in (run_root / "run.log").read_text(encoding="utf-8")


def test_product_app_startup_reconciles_orphaned_cancel_requested_run(tmp_path: Path) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)

    run_id = "run-orphaned-cancel"
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    server._record_run_start(
        run_id=run_id,
        flow_name="detail.dot",
        working_directory=str(workdir),
        model="gpt-detail",
    )
    server._record_run_status(run_id, "cancel_requested", "cancel_requested_by_user")
    run_root = server._run_root(run_id)
    _write_checkpoint(run_root, current_node="review", completed_nodes=["start"])

    with TestClient(product_app.app) as client:
        response = client.get(f"/attractor/pipelines/{run_id}")

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "canceled"
    assert payload["outcome"] is None
    assert payload["last_error"] == (
        "Run was interrupted when the Attractor server stopped before cancellation completed."
    )
    assert payload["ended_at"]
