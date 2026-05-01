from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

import httpx
import pytest

from attractor.execution import (
    ExecutionLaunchError,
    ExecutionProtocolError,
    RemoteWorkerClient,
    WorkerAPIError,
    WorkerCallbackRequest,
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerEvent,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerProfile,
    WorkerRunAdmissionRequest,
    WorkerRunAdmissionResponse,
    WorkerRunSnapshot,
)


def _worker() -> WorkerProfile:
    return WorkerProfile(
        id="worker-a",
        label="Worker A",
        base_url="https://worker.example.test",
        auth_token_env="SPARK_WORKER_TOKEN",
    )


def _client(
    monkeypatch: pytest.MonkeyPatch,
    handler: httpx.MockTransport,
    *,
    token: str = "secret-token",
) -> RemoteWorkerClient:
    monkeypatch.setenv("SPARK_WORKER_TOKEN", token)
    return RemoteWorkerClient(_worker(), client=httpx.Client(transport=handler, base_url=_worker().base_url))


def _json_response(payload: dict[str, Any], status_code: int = 200) -> httpx.Response:
    return httpx.Response(status_code, json=payload)


def _health_payload(**overrides: Any) -> dict[str, Any]:
    return {
        "worker_id": "stable-worker",
        "worker_version": "1.2.3",
        "protocol_version": "v1",
        "status": "ok",
        "capabilities": {"shell": True},
        **overrides,
    }


def _info_payload(**overrides: Any) -> dict[str, Any]:
    return {
        "worker_id": "stable-worker",
        "worker_version": "1.2.3",
        "protocol_version": "v1",
        "status": "ok",
        "capabilities": {"shell": True},
        "supported_images": ["spark-worker:latest"],
        "policies": {"images": "allow-list"},
        "resource_labels": {"region": "iad"},
        **overrides,
    }


def test_remote_client_fails_before_network_contact_when_auth_token_env_is_missing(monkeypatch: pytest.MonkeyPatch) -> None:
    requests: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        return _json_response(_health_payload())

    monkeypatch.delenv("SPARK_WORKER_TOKEN", raising=False)

    with pytest.raises(ExecutionLaunchError, match="SPARK_WORKER_TOKEN"):
        RemoteWorkerClient(_worker(), client=httpx.Client(transport=httpx.MockTransport(handler)))

    assert requests == []


def test_remote_client_fails_before_network_contact_when_auth_token_env_is_empty(monkeypatch: pytest.MonkeyPatch) -> None:
    requests: list[httpx.Request] = []
    monkeypatch.setenv("SPARK_WORKER_TOKEN", "  ")

    with pytest.raises(ExecutionLaunchError):
        RemoteWorkerClient(
            _worker(),
            client=httpx.Client(
                transport=httpx.MockTransport(lambda request: requests.append(request) or _json_response(_health_payload()))
            ),
        )

    assert requests == []


def test_remote_client_sends_bearer_auth_on_every_rest_request(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        captured.append(request)
        if request.url.path == "/v1/health":
            return _json_response(_health_payload())
        if request.url.path == "/v1/worker-info":
            return _json_response(_info_payload())
        if request.url.path == "/v1/runs" and request.method == "POST":
            return _json_response(
                {
                    "run_id": "run-1",
                    "worker_id": "stable-worker",
                    "status": "preparing",
                    "event_url": "/v1/runs/run-1/events",
                    "last_sequence": 2,
                    "accepted": True,
                },
                202,
            )
        if request.url.path == "/v1/runs/run-1" and request.method == "GET":
            return _json_response(
                {
                    "run_id": "run-1",
                    "status": "ready",
                    "execution_profile_id": "remote",
                    "protocol_version": "v1",
                    "worker_id": "stable-worker",
                    "worker_version": "1.2.3",
                    "mapped_project_path": "/srv/project",
                    "last_sequence": 2,
                }
            )
        if request.url.path == "/v1/runs/run-1/nodes":
            return _json_response(
                {"run_id": "run-1", "node_execution_id": "node-exec-1", "node_id": "n1", "attempt": 1, "status": "accepted"},
                202,
            )
        if request.url.path.endswith("/answer") or request.url.path.endswith("/result"):
            request_id = request.url.path.split("/")[-2]
            return _json_response({"run_id": "run-1", "request_id": request_id, "status": "accepted"})
        if request.url.path == "/v1/runs/run-1/cancel":
            return _json_response({"run_id": "run-1", "status": "canceling"})
        if request.url.path == "/v1/runs/run-1" and request.method == "DELETE":
            return _json_response({"run_id": "run-1", "status": "closed", "deleted": True})
        raise AssertionError(f"unexpected request {request.method} {request.url.path}")

    client = _client(monkeypatch, httpx.MockTransport(handler))

    assert isinstance(client.health(), WorkerHealthResponse)
    assert isinstance(client.worker_info(), WorkerInfoResponse)
    assert isinstance(
        client.admit_run(
            WorkerRunAdmissionRequest(
                run_id="run-1",
                execution_profile_id="remote",
                mapped_project_path="/srv/project",
            )
        ),
        WorkerRunAdmissionResponse,
    )
    assert isinstance(client.get_run("run-1"), WorkerRunSnapshot)
    assert isinstance(client.submit_node("run-1", WorkerNodeRequest(node_id="n1")), WorkerNodeAcceptedResponse)
    callback = WorkerCallbackRequest(payload={"value": "ok"})
    assert client.answer_human_gate("run-1", "gate-1", callback).request_id == "gate-1"
    assert client.child_run_result("run-1", "child-1", callback).request_id == "child-1"
    assert client.child_status_result("run-1", "status-1", callback).request_id == "status-1"
    assert isinstance(client.cancel_run("run-1"), WorkerCancelResponse)
    assert isinstance(client.cleanup_run("run-1"), WorkerCleanupResponse)

    assert captured
    assert {request.headers["authorization"] for request in captured} == {"Bearer secret-token"}


def test_remote_client_sends_bearer_auth_on_sse_stream_request(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: list[httpx.Request] = []
    event = WorkerEvent(
        run_id="run-1",
        sequence=7,
        event_type="node_result",
        timestamp=datetime.now(timezone.utc),
        worker_id="stable-worker",
        execution_profile_id="remote",
        payload={"ok": True},
    )

    def handler(request: httpx.Request) -> httpx.Response:
        captured.append(request)
        body = f"id: 7\nevent: node_result\ndata: {event.model_dump_json()}\n\n"
        return httpx.Response(200, content=body, headers={"content-type": "text/event-stream"})

    client = _client(monkeypatch, httpx.MockTransport(handler))

    events = list(client.stream_events("run-1", after=6, last_event_id="5"))

    assert [stream_event.sequence for stream_event in events] == [7]
    assert captured[0].url.path == "/v1/runs/run-1/events"
    assert captured[0].url.params["after"] == "6"
    assert captured[0].headers["authorization"] == "Bearer secret-token"
    assert captured[0].headers["last-event-id"] == "5"


@pytest.mark.parametrize("method_name", ["health", "worker_info"])
def test_remote_client_rejects_incompatible_worker_protocol(
    monkeypatch: pytest.MonkeyPatch,
    method_name: str,
) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        payload = _health_payload(protocol_version="v2") if request.url.path == "/v1/health" else _info_payload(protocol_version="v2")
        return _json_response(payload)

    client = _client(monkeypatch, httpx.MockTransport(handler))

    with pytest.raises(ExecutionProtocolError, match="unsupported protocol"):
        getattr(client, method_name)()


@pytest.mark.parametrize("method_name", ["health", "worker_info"])
def test_remote_client_rejects_missing_worker_protocol(
    monkeypatch: pytest.MonkeyPatch,
    method_name: str,
) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        payload = _health_payload() if request.url.path == "/v1/health" else _info_payload()
        payload.pop("protocol_version")
        return _json_response(payload)

    client = _client(monkeypatch, httpx.MockTransport(handler))

    with pytest.raises(ExecutionProtocolError, match="did not advertise protocol metadata"):
        getattr(client, method_name)()


@pytest.mark.parametrize("method_name", ["health", "worker_info"])
def test_remote_client_rejects_worker_metadata_without_capability_metadata(
    monkeypatch: pytest.MonkeyPatch,
    method_name: str,
) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        payload = _health_payload() if request.url.path == "/v1/health" else _info_payload()
        payload.pop("capabilities")
        return _json_response(payload)

    client = _client(monkeypatch, httpx.MockTransport(handler))

    with pytest.raises(ExecutionProtocolError, match="capability metadata"):
        getattr(client, method_name)()


@pytest.mark.parametrize("method_name", ["health", "worker_info"])
def test_remote_client_accepts_explicit_v1_worker_protocol(
    monkeypatch: pytest.MonkeyPatch,
    method_name: str,
) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        payload = _health_payload(protocol_version="v1") if request.url.path == "/v1/health" else _info_payload(protocol_version="v1")
        return _json_response(payload)

    client = _client(monkeypatch, httpx.MockTransport(handler))

    assert getattr(client, method_name)().protocol_version == "v1"


def test_remote_client_preserves_structured_worker_error_response(monkeypatch: pytest.MonkeyPatch) -> None:
    client = _client(
        monkeypatch,
        httpx.MockTransport(
            lambda request: _json_response(
                {
                    "error": {
                        "code": "unsupported_image",
                        "message": "Image is not allowed.",
                        "retryable": False,
                        "details": {"image": "other:latest"},
                    }
                },
                400,
            )
        ),
    )

    with pytest.raises(WorkerAPIError) as exc_info:
        client.admit_run(
            WorkerRunAdmissionRequest(
                run_id="run-1",
                execution_profile_id="remote",
                image="other:latest",
                mapped_project_path="/srv/project",
            )
        )

    assert exc_info.value.code == "unsupported_image"
    assert exc_info.value.message == "Image is not allowed."
    assert exc_info.value.status_code == 400
    assert exc_info.value.details == {"image": "other:latest"}


def test_remote_client_maps_transport_failures_to_execution_launch_error(monkeypatch: pytest.MonkeyPatch) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("connection refused", request=request)

    client = _client(monkeypatch, httpx.MockTransport(handler))

    with pytest.raises(ExecutionLaunchError, match="connection refused"):
        client.health()
