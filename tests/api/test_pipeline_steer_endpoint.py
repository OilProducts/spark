from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine import Checkpoint, save_checkpoint
from attractor.handlers.base import ChildInterventionRequest, ChildInterventionResult


class _InterventionRunner:
    def __init__(self, *, delivery_mode: str = "test") -> None:
        self.delivery_mode = delivery_mode
        self.requests: list[ChildInterventionRequest] = []

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        self.requests.append(request)
        return ChildInterventionResult(
            run_id=request.child_run_id,
            status="delivered",
            delivery_mode=self.delivery_mode,
            reason=request.reason,
            message="queued",
            target_node_id=request.target_node_id,
        )


def _seed_run_root(run_id: str, workdir: Path) -> Path:
    workdir.mkdir(parents=True, exist_ok=True)
    return server._ensure_run_root_for_project(run_id, str(workdir))


def _seed_active_run(run_id: str, workdir: Path, runner: object) -> None:
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS[run_id] = server.ActiveRun(
            run_id=run_id,
            flow_name=f"{run_id}.dot",
            working_directory=str(workdir),
            model="test-model",
            status="running",
            runner=runner,
        )


def test_pipeline_steer_rejects_empty_message(attractor_api_client: TestClient) -> None:
    response = attractor_api_client.post("/pipelines/run-1/steer", json={"message": "  "})

    assert response.status_code == 200
    assert response.json() == {
        "status": "validation_error",
        "error": "message is required.",
    }


def test_pipeline_steer_defaults_to_active_child_from_parent_checkpoint(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    parent_root = _seed_run_root("parent-1", tmp_path / "parent-work")
    save_checkpoint(
        parent_root / "state.json",
        Checkpoint(
            active_node="manager",
            context={
                "context.stack.child.run_id": "child-1",
                "context.stack.child.active_stage": "task",
            },
        ),
    )
    child_runner = _InterventionRunner(delivery_mode="child-backend")
    _seed_active_run("child-1", tmp_path / "child-work", child_runner)

    response = attractor_api_client.post(
        "/pipelines/parent-1/steer",
        json={"message": "Please address the current failure."},
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["target_run_id"] == "child-1"
    assert payload["run_id"] == "child-1"
    assert payload["status"] == "delivered"
    assert payload["delivery_mode"] == "child-backend"
    assert child_runner.requests[0].child_run_id == "child-1"
    assert child_runner.requests[0].target_node_id == "task"
    assert child_runner.requests[0].message == "Please address the current failure."
    event = server.EVENT_HUB.history("parent-1")[-1]
    assert event["type"] == "HumanInterventionRequested"
    assert event["target_run_id"] == "child-1"
    assert event["target_node_id"] == "task"
    assert event["intervention_status"] == "delivered"


def test_pipeline_steer_explicit_target_overrides_default_child(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    parent_root = _seed_run_root("parent-1", tmp_path / "parent-work")
    save_checkpoint(
        parent_root / "state.json",
        Checkpoint(
            active_node="manager",
            context={
                "context.stack.child.run_id": "child-1",
                "context.stack.child.active_stage": "task",
            },
        ),
    )
    default_runner = _InterventionRunner(delivery_mode="default-child")
    explicit_runner = _InterventionRunner(delivery_mode="explicit-child")
    _seed_active_run("child-1", tmp_path / "child-work", default_runner)
    _seed_active_run("child-2", tmp_path / "child-two-work", explicit_runner)

    response = attractor_api_client.post(
        "/pipelines/parent-1/steer",
        json={
            "message": "Use the explicit target.",
            "target_run_id": "child-2",
            "target_node_id": "review",
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["target_run_id"] == "child-2"
    assert payload["run_id"] == "child-2"
    assert payload["delivery_mode"] == "explicit-child"
    assert default_runner.requests == []
    assert explicit_runner.requests[0].target_node_id == "review"


def test_pipeline_steer_defaults_to_url_pipeline_when_no_active_child_exists(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    _seed_run_root("parent-1", tmp_path / "parent-work")
    parent_runner = _InterventionRunner(delivery_mode="parent-backend")
    _seed_active_run("parent-1", tmp_path / "parent-work", parent_runner)

    response = attractor_api_client.post(
        "/pipelines/parent-1/steer",
        json={"message": "Steer the parent run."},
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["target_run_id"] == "parent-1"
    assert payload["run_id"] == "parent-1"
    assert payload["delivery_mode"] == "parent-backend"
    assert parent_runner.requests[0].child_run_id == "parent-1"


def test_pipeline_steer_records_rejected_result_for_inactive_target(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    _seed_run_root("parent-1", tmp_path / "parent-work")

    response = attractor_api_client.post(
        "/pipelines/parent-1/steer",
        json={"message": "Please steer.", "target_run_id": "missing-child"},
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["target_run_id"] == "missing-child"
    assert payload["run_id"] == "missing-child"
    assert payload["status"] == "rejected"
    assert payload["reason"] == "no_active_child_run"
    assert server.EVENT_HUB.history("parent-1")[-1]["type"] == "HumanInterventionRequested"
