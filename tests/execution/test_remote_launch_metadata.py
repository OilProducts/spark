from __future__ import annotations

from pathlib import Path

import pytest

from attractor.execution import (
    ExecutionLaunchError,
    ExecutionProfile,
    ExecutionProtocolError,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerProfile,
    WorkerRunAdmissionResponse,
    admit_remote_launch,
)


def _profile(tmp_path: Path, capabilities: object = None) -> ExecutionProfile:
    return ExecutionProfile(
        id="remote-fast",
        mode="remote_worker",
        worker_id="worker-a",
        image="spark-worker:latest",
        control_project_root=tmp_path / "control",
        worker_project_root=Path("/srv/projects"),
        worker_runtime_root=Path("/srv/runtime"),
        capabilities=capabilities or {"shell": True},
    )


def _worker() -> WorkerProfile:
    return WorkerProfile(
        id="worker-a",
        label="Worker A",
        base_url="https://worker.example.test",
        auth_token_env="SPARK_WORKER_TOKEN",
    )


class _Client:
    def __init__(
        self,
        *,
        health: WorkerHealthResponse | None = None,
        info: WorkerInfoResponse | None = None,
    ) -> None:
        self.health_response = health or WorkerHealthResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )
        self.info_response = info or WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )
        self.admission_requests = []

    def __enter__(self) -> "_Client":
        return self

    def __exit__(self, *_exc_info: object) -> None:
        return None

    def health(self) -> WorkerHealthResponse:
        return self.health_response

    def worker_info(self) -> WorkerInfoResponse:
        return self.info_response

    def admit_run(self, request):
        self.admission_requests.append(request)
        return WorkerRunAdmissionResponse(
            run_id=request.run_id,
            worker_id=self.info_response.worker_id,
            status="preparing",
            event_url=f"/v1/runs/{request.run_id}/events",
            last_sequence=0,
            accepted=True,
        )


def test_remote_launch_admission_maps_paths_and_sends_worker_metadata(tmp_path: Path) -> None:
    client = _Client()

    result = admit_remote_launch(
        _profile(tmp_path),
        _worker(),
        run_id="run-1",
        control_project_path=tmp_path / "control" / "project",
        client_factory=lambda worker: client,
    )

    assert result.metadata.mapped_worker_project_path == "/srv/projects/project"
    assert result.metadata.worker_runtime_root == "/srv/runtime"
    assert len(client.admission_requests) == 1
    request = client.admission_requests[0]
    assert request.execution_profile_id == "remote-fast"
    assert request.image == "spark-worker:latest"
    assert request.mapped_project_path == "/srv/projects/project"
    assert request.worker_runtime_root == "/srv/runtime"
    assert request.capabilities == {"shell": True}
    assert request.metadata["worker_id"] == "worker-a"
    assert not (tmp_path / "control").exists()


def test_remote_launch_rejects_inconsistent_worker_identity_before_admission(tmp_path: Path) -> None:
    client = _Client(
        info=WorkerInfoResponse(
            worker_id="worker-b",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )
    )

    with pytest.raises(ExecutionProtocolError, match="inconsistent worker identity"):
        admit_remote_launch(
            _profile(tmp_path),
            _worker(),
            run_id="run-1",
            control_project_path=tmp_path / "control" / "project",
            client_factory=lambda worker: client,
        )

    assert client.admission_requests == []


def test_remote_launch_rejects_mismatched_configured_worker_identity_before_admission(tmp_path: Path) -> None:
    client = _Client(
        health=WorkerHealthResponse(
            worker_id="worker-b",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        ),
        info=WorkerInfoResponse(
            worker_id="worker-b",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        ),
    )

    with pytest.raises(ExecutionProtocolError, match="mismatched configured worker identity"):
        admit_remote_launch(
            _profile(tmp_path),
            _worker(),
            run_id="run-1",
            control_project_path=tmp_path / "control" / "project",
            client_factory=lambda worker: client,
        )

    assert client.admission_requests == []


def test_remote_launch_rejects_missing_required_capability_before_admission(tmp_path: Path) -> None:
    client = _Client(
        info=WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": False},
        )
    )

    with pytest.raises(ExecutionProtocolError, match="missing required launch capabilities: shell"):
        admit_remote_launch(
            _profile(tmp_path),
            _worker(),
            run_id="run-1",
            control_project_path=tmp_path / "control" / "project",
            client_factory=lambda worker: client,
        )

    assert client.admission_requests == []


def test_remote_launch_rejects_outside_project_path_before_worker_contact(tmp_path: Path) -> None:
    contacted = False

    class ContactRecordingClient(_Client):
        def health(self) -> WorkerHealthResponse:
            nonlocal contacted
            contacted = True
            return super().health()

    client = ContactRecordingClient()

    with pytest.raises(ExecutionLaunchError, match="outside remote control_project_root"):
        admit_remote_launch(
            _profile(tmp_path),
            _worker(),
            run_id="run-outside-root",
            control_project_path=tmp_path / "control-other" / "project",
            client_factory=lambda worker: client,
        )

    assert contacted is False
    assert client.admission_requests == []
    assert not (tmp_path / "control-other").exists()
