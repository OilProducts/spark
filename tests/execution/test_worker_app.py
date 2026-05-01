from __future__ import annotations

import json
import socket
import threading
import time
from typing import Any

from fastapi.testclient import TestClient
import httpx
import pytest
import uvicorn

from attractor.execution.errors import WorkerAPIError
from attractor.execution.worker_app import create_worker_app
from attractor.execution.worker_models import WORKER_EVENT_TYPES
from attractor.execution.worker_runtime import InProcessWorkerRuntime


AUTH = {"Authorization": "Bearer test-token"}


def _read_sse(
    client: TestClient,
    path: str,
    headers: dict[str, str] | None = None,
    *,
    expected_count: int,
) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    separator = "&" if "?" in path else "?"
    path = f"{path}{separator}limit={expected_count}"
    with client.stream("GET", path, headers=headers or AUTH) as stream:
        block_lines: list[str] = []
        for line in stream.iter_lines():
            if line:
                block_lines.append(line)
                continue
            if block_lines:
                events.append(_parse_sse_block(block_lines))
                block_lines = []
                if len(events) == expected_count:
                    break
    return events


def _parse_sse_block(lines: list[str]) -> dict[str, object]:
    fields: dict[str, object] = {}
    for line in lines:
        name, value = line.split(": ", 1)
        fields[name] = json.loads(value) if name == "data" else value
    return fields


def _read_one_live_sse(
    base_url: str,
    path: str,
    ready: threading.Event,
    result: list[dict[str, object]],
) -> None:
    with httpx.Client(base_url=base_url, timeout=5.0) as client:
        with client.stream("GET", path, headers=AUTH) as stream:
            stream.raise_for_status()
            ready.set()
            block_lines: list[str] = []
            for line in stream.iter_lines():
                if line:
                    block_lines.append(line)
                    continue
                if block_lines:
                    result.append(_parse_sse_block(block_lines))
                    return


def _unused_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _start_test_server(app) -> tuple[uvicorn.Server, threading.Thread, str]:
    port = _unused_tcp_port()
    server = uvicorn.Server(
        uvicorn.Config(app, host="127.0.0.1", port=port, log_level="warning", lifespan="off")
    )
    thread = threading.Thread(target=server.run)
    thread.start()
    deadline = time.monotonic() + 5
    while not server.started and thread.is_alive() and time.monotonic() < deadline:
        time.sleep(0.01)
    if not server.started:
        server.should_exit = True
        thread.join(timeout=2)
        raise AssertionError("test server did not start")
    return server, thread, f"http://127.0.0.1:{port}"


def _stop_test_server(server: uvicorn.Server, thread: threading.Thread) -> None:
    server.should_exit = True
    thread.join(timeout=5)


def _wait_for_snapshot_event(client: TestClient, run_id: str, event_type: str, *, timeout: float = 2.0) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_snapshot: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        last_snapshot = client.get(f"/v1/runs/{run_id}", headers=AUTH).json()
        for event in last_snapshot["events"]:
            if event["event_type"] == event_type:
                return event
        time.sleep(0.01)
    raise AssertionError(f"event {event_type!r} not observed; snapshot={last_snapshot!r}")


def test_worker_v1_endpoints_require_bearer_auth() -> None:
    client = TestClient(create_worker_app(token="test-token"))

    missing = client.get("/v1/health")
    malformed = client.get("/v1/health", headers={"Authorization": "Token test-token"})
    invalid = client.get("/v1/health", headers={"Authorization": "Bearer wrong"})

    assert missing.status_code == 401
    assert missing.json() == {
        "error": {
            "code": "unauthorized",
            "message": "Missing or malformed bearer credentials.",
            "retryable": False,
            "details": {"scheme": "Bearer"},
        }
    }
    assert malformed.status_code == 401
    assert malformed.json()["error"]["code"] == "unauthorized"
    assert invalid.status_code == 403
    assert invalid.json()["error"] == {
        "code": "forbidden",
        "message": "Bearer credentials were rejected.",
        "retryable": False,
        "details": {},
    }


@pytest.mark.parametrize(
    ("method", "path", "json_body"),
    [
        ("GET", "/v1/health", None),
        ("GET", "/v1/worker-info", None),
        ("POST", "/v1/runs", {"run_id": "run-auth", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"}),
        ("GET", "/v1/runs/run-auth", None),
        ("POST", "/v1/runs/run-auth/nodes", {"node_id": "n1"}),
        ("GET", "/v1/runs/run-auth/events", None),
        ("POST", "/v1/runs/run-auth/human-gates/g1/answer", {"payload": {}}),
        ("POST", "/v1/runs/run-auth/child-runs/c1/result", {"payload": {}}),
        ("POST", "/v1/runs/run-auth/child-status/s1/result", {"payload": {}}),
        ("POST", "/v1/runs/run-auth/cancel", None),
        ("DELETE", "/v1/runs/run-auth", None),
    ],
)
def test_every_worker_v1_endpoint_rejects_missing_bearer_credentials(
    method: str,
    path: str,
    json_body: dict[str, object] | None,
) -> None:
    client = TestClient(create_worker_app(token="test-token"))

    response = client.request(method, path, json=json_body)

    assert response.status_code == 401
    assert response.json()["error"]["code"] == "unauthorized"


def test_health_and_worker_info_return_compatibility_metadata() -> None:
    client = TestClient(
        create_worker_app(
            token="test-token",
            worker_id="worker-a",
            worker_version="1.2.3",
            capabilities={"shell": True},
            supported_images=["spark-worker:latest"],
            policies={"images": "allow-list"},
            resource_labels={"region": "iad"},
            diagnostics={"load": "low"},
        )
    )

    health = client.get("/v1/health", headers=AUTH)
    info = client.get("/v1/worker-info", headers=AUTH)

    assert health.status_code == 200
    assert health.json() == {
        "worker_id": "worker-a",
        "worker_version": "1.2.3",
        "protocol_version": "v1",
        "status": "ok",
        "capabilities": {"shell": True},
        "diagnostics": {"load": "low"},
    }
    assert info.status_code == 200
    payload = info.json()
    assert payload["worker_id"] == "worker-a"
    assert payload["worker_version"] == "1.2.3"
    assert payload["protocol_version"] == "v1"
    assert payload["capabilities"] == {"shell": True}
    assert payload["supported_images"] == ["spark-worker:latest"]
    assert payload["policies"] == {"images": "allow-list"}
    assert payload["resource_labels"] == {"region": "iad"}
    assert payload["validation_details"] == {"profile_selection": "control_plane_owned"}


def test_worker_run_lifecycle_endpoints_are_idempotent_and_conflict_on_different_repeats() -> None:
    client = TestClient(
        create_worker_app(
            token="test-token",
            worker_id="worker-a",
            worker_version="1.2.3",
            capabilities={"shell": True},
        )
    )
    request = {
        "run_id": "run-1",
        "execution_profile_id": "remote-dev",
        "protocol_version": "v1",
        "image": "spark-worker:latest",
        "mapped_project_path": "/srv/projects/acme",
        "worker_runtime_root": "/srv/runtime",
        "capabilities": {"shell": True},
        "metadata": {"source": "test"},
    }

    admitted = client.post("/v1/runs", json=request, headers=AUTH)
    repeated = client.post("/v1/runs", json=request, headers=AUTH)
    conflict = client.post("/v1/runs", json={**request, "image": "other:latest"}, headers=AUTH)

    assert admitted.status_code == 202
    assert admitted.json() == {
        "run_id": "run-1",
        "worker_id": "worker-a",
        "status": "preparing",
        "event_url": "/v1/runs/run-1/events",
        "last_sequence": 2,
        "accepted": True,
    }
    assert repeated.status_code == 202
    assert repeated.json() == admitted.json()
    assert conflict.status_code == 409
    assert conflict.json()["error"]["code"] == "conflict"

    snapshot = client.get("/v1/runs/run-1", headers=AUTH)
    assert snapshot.status_code == 200
    snapshot_payload = snapshot.json()
    assert snapshot_payload["status"] == "ready"
    assert snapshot_payload["worker_id"] == "worker-a"
    assert snapshot_payload["worker_version"] == "1.2.3"
    assert snapshot_payload["worker_capabilities"] == {"shell": True}
    assert snapshot_payload["runtime_id"] == "runtime-run-1"
    assert snapshot_payload["container_id"] == "container-run-1"
    assert snapshot_payload["runtime"]["worker_project_path"] == "/srv/projects/acme"
    assert snapshot_payload["last_sequence"] == 6
    assert snapshot_payload["last_error"] is None
    assert [event["event_type"] for event in snapshot_payload["events"]] == [
        "run_started",
        "run_preparing",
        "image_pull_started",
        "image_pull_progress",
        "container_creating",
        "run_ready",
    ]
    assert [event["sequence"] for event in snapshot_payload["events"]] == [1, 2, 3, 4, 5, 6]


def test_worker_run_admission_validates_configured_policy_compatibility(tmp_path) -> None:
    mapped_root = tmp_path / "mapped"
    mapped_root.mkdir()
    client = TestClient(
        create_worker_app(
            token="test-token",
            worker_id="worker-a",
            capabilities={"shell": True, "network": False},
            supported_images=["spark-worker:latest"],
            policies={
                "mapped_project_path_prefixes": [str(mapped_root)],
                "require_existing_mapped_project_path": True,
                "required_capabilities": {"shell": True},
                "unavailable_resources": {"gpu": "a100"},
            },
        )
    )
    base_request = {
        "run_id": "run-policy",
        "execution_profile_id": "remote-dev",
        "protocol_version": "v1",
        "image": "spark-worker:latest",
        "mapped_project_path": str(mapped_root),
        "capabilities": {"shell": True, "network": False},
    }

    accepted = client.post("/v1/runs", json=base_request, headers=AUTH)
    bad_protocol = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "bad-protocol", "protocol_version": "v2"},
        headers=AUTH,
    )
    bad_image = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "bad-image", "image": "other:latest"},
        headers=AUTH,
    )
    bad_path = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "bad-path", "mapped_project_path": str(tmp_path / "other")},
        headers=AUTH,
    )
    unavailable_path = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "unavailable-path", "mapped_project_path": str(mapped_root / "missing")},
        headers=AUTH,
    )
    bad_capability = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "bad-capability", "capabilities": {"shell": True, "network": True}},
        headers=AUTH,
    )
    missing_required = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "missing-required", "capabilities": {"network": False}},
        headers=AUTH,
    )
    unavailable_resource = client.post(
        "/v1/runs",
        json={**base_request, "run_id": "unavailable-resource", "resources": {"gpu": "a100"}},
        headers=AUTH,
    )

    assert accepted.status_code == 202
    assert accepted.json()["status"] == "preparing"
    assert accepted.json()["worker_id"] == "worker-a"
    assert accepted.json()["last_sequence"] == 2
    assert bad_protocol.status_code == 400
    assert bad_protocol.json()["error"]["code"] == "unsupported_protocol"
    assert bad_image.status_code == 400
    assert bad_image.json()["error"]["code"] == "unsupported_image"
    assert bad_path.status_code == 400
    assert bad_path.json()["error"]["code"] == "mapped_path_denied"
    assert unavailable_path.status_code == 400
    assert unavailable_path.json()["error"]["code"] == "mapped_path_unavailable"
    assert unavailable_path.json()["error"]["retryable"] is True
    assert bad_capability.status_code == 400
    assert bad_capability.json()["error"]["code"] == "unsupported_capabilities"
    assert missing_required.status_code == 400
    assert missing_required.json()["error"]["code"] == "missing_required_capabilities"
    assert unavailable_resource.status_code == 409
    assert unavailable_resource.json()["error"]["code"] == "resource_unavailable"
    assert unavailable_resource.json()["error"]["retryable"] is True


def test_worker_preparation_failure_emits_run_failed_before_nodes_execute() -> None:
    client = TestClient(
        create_worker_app(
            token="test-token",
            worker_id="worker-a",
            policies={"image_pull_failures": ["missing:latest"]},
        )
    )
    run_request = {
        "run_id": "run-prep-fails",
        "execution_profile_id": "remote-dev",
        "image": "missing:latest",
        "mapped_project_path": "/srv/projects/acme",
    }

    admitted = client.post("/v1/runs", json=run_request, headers=AUTH)
    node = client.post("/v1/runs/run-prep-fails/nodes", json={"node_id": "n1"}, headers=AUTH)
    snapshot = client.get("/v1/runs/run-prep-fails", headers=AUTH)

    assert admitted.status_code == 202
    assert admitted.json()["status"] == "preparing"
    assert node.status_code == 409
    assert node.json()["error"]["code"] == "run_not_ready"
    payload = snapshot.json()
    assert payload["status"] == "failed"
    assert payload["runtime"] is None
    assert payload["last_error"]["code"] == "image_unavailable"
    assert [event["event_type"] for event in payload["events"]] == [
        "run_started",
        "run_preparing",
        "image_pull_started",
        "image_pull_progress",
        "container_creating",
        "run_failed",
    ]
    assert payload["events"][-1]["payload"] == {
        "code": "image_unavailable",
        "message": "Requested image could not be pulled or found on this worker.",
        "retryable": True,
        "details": {"image": "missing:latest"},
    }
    assert payload["nodes"] == {}


def test_worker_node_callbacks_events_cancel_and_cleanup_use_public_api() -> None:
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a"))
    run_request = {
        "run_id": "run-2",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }
    node_request = {"node_id": "implement", "attempt": 2, "payload": {"input": "value"}}

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    node = client.post("/v1/runs/run-2/nodes", json=node_request, headers=AUTH)
    duplicate_node = client.post("/v1/runs/run-2/nodes", json=node_request, headers=AUTH)
    conflicting_node = client.post(
        "/v1/runs/run-2/nodes",
        json={**node_request, "payload": {"input": "different"}},
        headers=AUTH,
    )
    gate = client.post("/v1/runs/run-2/human-gates/gate-1/answer", json={"payload": {"answer": "yes"}}, headers=AUTH)
    child = client.post("/v1/runs/run-2/child-runs/child-1/result", json={"payload": {"status": "done"}}, headers=AUTH)
    child_status = client.post(
        "/v1/runs/run-2/child-status/status-1/result",
        json={"payload": {"status": "running"}},
        headers=AUTH,
    )
    callback_conflict = client.post(
        "/v1/runs/run-2/human-gates/gate-1/answer",
        json={"payload": {"answer": "no"}},
        headers=AUTH,
    )
    cancel = client.post("/v1/runs/run-2/cancel", headers=AUTH)
    node_after_cancel = client.post("/v1/runs/run-2/nodes", json={"node_id": "after-cancel"}, headers=AUTH)

    assert node.status_code == 202
    assert node.json() == {
        "run_id": "run-2",
        "node_execution_id": "implement:2",
        "node_id": "implement",
        "attempt": 2,
        "status": "accepted",
    }
    assert duplicate_node.status_code == 202
    assert conflicting_node.status_code == 409
    assert gate.json() == {"run_id": "run-2", "request_id": "gate-1", "status": "accepted"}
    assert child.json() == {"run_id": "run-2", "request_id": "child-1", "status": "accepted"}
    assert child_status.json() == {"run_id": "run-2", "request_id": "status-1", "status": "accepted"}
    assert callback_conflict.status_code == 409
    assert cancel.json() == {"run_id": "run-2", "status": "canceled"}
    assert node_after_cancel.status_code == 409
    assert node_after_cancel.json()["error"]["code"] == "run_not_ready"

    events = _read_sse(client, "/v1/runs/run-2/events?after=4", expected_count=4)
    assert [event["id"] for event in events] == ["5", "6", "7", "8"]
    assert [event["event"] for event in events] == ["node_started", "node_result", "run_canceling", "run_canceled"]
    event_payload = events[0]["data"]
    assert event_payload["run_id"] == "run-2"
    assert event_payload["sequence"] == 5
    assert event_payload["event_type"] == "node_started"
    assert event_payload["worker_id"] == "worker-a"
    assert event_payload["execution_profile_id"] == "remote-dev"
    assert event_payload["node_id"] == "implement"
    assert event_payload["node_attempt"] == 2

    cleanup = client.delete("/v1/runs/run-2", headers=AUTH)
    repeated_cleanup = client.delete("/v1/runs/run-2", headers=AUTH)
    assert cleanup.json() == {"run_id": "run-2", "status": "closed", "deleted": True}
    assert repeated_cleanup.json() == {"run_id": "run-2", "status": "closed", "deleted": False}


@pytest.mark.parametrize(
    ("path", "callback_key", "request_id"),
    [
        ("/v1/runs/run-callbacks/human-gates/gate-1/answer", "human_gate:gate-1", "gate-1"),
        ("/v1/runs/run-callbacks/child-runs/child-1/result", "child_run:child-1", "child-1"),
        ("/v1/runs/run-callbacks/child-status/status-1/result", "child_status:status-1", "status-1"),
    ],
)
def test_worker_callback_deliveries_are_idempotent_and_conflict_by_request_id(
    path: str,
    callback_key: str,
    request_id: str,
) -> None:
    client = TestClient(create_worker_app(token="test-token"))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-callbacks", "execution_profile_id": "remote-dev", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    first = client.post(path, json={"payload": {"value": "ready", "metadata": {"source": "control-plane"}}}, headers=AUTH)
    repeat = client.post(path, json={"payload": {"value": "ready", "metadata": {"source": "control-plane"}}}, headers=AUTH)
    conflict = client.post(path, json={"payload": {"value": "different"}}, headers=AUTH)
    snapshot = client.get("/v1/runs/run-callbacks", headers=AUTH)

    assert first.status_code == 200
    assert first.json() == {"run_id": "run-callbacks", "request_id": request_id, "status": "accepted"}
    assert repeat.status_code == 200
    assert repeat.json() == first.json()
    assert conflict.status_code == 409
    assert conflict.json() == {
        "error": {
            "code": "conflict",
            "message": "Callback result conflicts with an earlier delivery.",
            "retryable": False,
            "details": {"callback_id": callback_key},
        }
    }
    assert snapshot.json()["callbacks"][callback_key] == {"value": "ready", "metadata": {"source": "control-plane"}}


@pytest.mark.parametrize(
    "path",
    [
        "/v1/runs/missing-run/human-gates/gate-1/answer",
        "/v1/runs/missing-run/child-runs/child-1/result",
        "/v1/runs/missing-run/child-status/status-1/result",
    ],
)
def test_worker_callback_deliveries_reject_unknown_runs(path: str) -> None:
    client = TestClient(create_worker_app(token="test-token"))

    response = client.post(path, json={"payload": {"value": "ready"}}, headers=AUTH)

    assert response.status_code == 404
    assert response.json()["error"] == {
        "code": "not_found",
        "message": "Worker run was not found.",
        "retryable": False,
        "details": {"run_id": "missing-run"},
    }


def test_worker_callback_delivery_for_unknown_request_is_recorded_for_later_resume() -> None:
    client = TestClient(create_worker_app(token="test-token"))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-early-callback", "execution_profile_id": "remote-dev", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    response = client.post(
        "/v1/runs/run-early-callback/child-runs/future-request/result",
        json={"payload": {"run_id": "child-99", "status": "completed"}},
        headers=AUTH,
    )
    snapshot = client.get("/v1/runs/run-early-callback", headers=AUTH)

    assert response.status_code == 200
    assert response.json() == {"run_id": "run-early-callback", "request_id": "future-request", "status": "accepted"}
    assert snapshot.json()["callbacks"]["child_run:future-request"] == {"run_id": "child-99", "status": "completed"}


def test_worker_events_replay_after_last_event_id_and_after_query_deterministically() -> None:
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a"))
    run_request = {
        "run_id": "run-replay",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    assert client.post("/v1/runs/run-replay/nodes", json={"node_id": "n1", "attempt": 1}, headers=AUTH).status_code == 202
    assert client.post("/v1/runs/run-replay/cancel", headers=AUTH).status_code == 200

    after_header = _read_sse(
        client,
        "/v1/runs/run-replay/events",
        headers={**AUTH, "Last-Event-ID": "4"},
        expected_count=3,
    )
    after_query = _read_sse(client, "/v1/runs/run-replay/events?after=4", expected_count=3)
    after_query_wins = _read_sse(
        client,
        "/v1/runs/run-replay/events?after=5",
        headers={**AUTH, "Last-Event-ID": "1"},
        expected_count=2,
    )

    assert after_header == after_query
    assert [event["id"] for event in after_query] == ["5", "6", "7"]
    assert [event["event"] for event in after_query] == ["node_started", "node_result", "run_canceling"]
    assert [event["id"] for event in after_query_wins] == ["6", "7"]


def test_worker_events_stream_delivers_events_appended_after_connection_opens() -> None:
    app = create_worker_app(token="test-token", worker_id="worker-a")
    server, server_thread, base_url = _start_test_server(app)
    run_request = {
        "run_id": "run-live",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }
    node_request = {"node_id": "live-node", "attempt": 2, "payload": {"step": "later"}}
    ready = threading.Event()
    result: list[dict[str, object]] = []
    errors: list[BaseException] = []

    try:
        with httpx.Client(base_url=base_url, timeout=5.0) as request_client:
            admitted = request_client.post("/v1/runs", json=run_request, headers=AUTH)
            assert admitted.status_code == 202

            def read_stream() -> None:
                try:
                    _read_one_live_sse(base_url, "/v1/runs/run-live/events?after=4", ready, result)
                except BaseException as exc:  # pragma: no cover - surfaced by the assertions below.
                    errors.append(exc)

            thread = threading.Thread(target=read_stream)
            thread.start()
            assert ready.wait(timeout=2)

            node = request_client.post("/v1/runs/run-live/nodes", json=node_request, headers=AUTH)
            thread.join(timeout=2)

            assert node.status_code == 202
            assert not thread.is_alive()
            assert errors == []
            assert len(result) == 1
            event = result[0]
            assert event["id"] == "5"
            assert event["event"] == "node_started"
            payload = event["data"]
            assert payload["run_id"] == "run-live"
            assert payload["sequence"] == 5
            assert payload["event_type"] == "node_started"
            assert payload["worker_id"] == "worker-a"
            assert payload["execution_profile_id"] == "remote-dev"
            assert payload["node_id"] == "live-node"
            assert payload["node_attempt"] == 2
            assert payload["payload"] == {"status": "started", "payload": {"step": "later"}, "context": {}}
    finally:
        _stop_test_server(server, server_thread)


def test_worker_events_stream_rejects_unauthenticated_sse_before_body_is_opened() -> None:
    client = TestClient(create_worker_app(token="test-token"))
    run_request = {
        "run_id": "run-auth-events",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    response = client.get("/v1/runs/run-auth-events/events")

    assert response.status_code == 401
    assert response.headers["content-type"].startswith("application/json")
    assert response.json()["error"]["code"] == "unauthorized"


def test_worker_event_store_can_emit_required_event_types_with_ordered_metadata() -> None:
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a"))
    run_request = {
        "run_id": "run-event-types",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    state = client.app.state.worker_state
    record = state.require_run("run-event-types")
    for event_type in WORKER_EVENT_TYPES:
        state.add_event(
            record,
            event_type,
            {"source": "test"},
            node_id="node-a" if event_type.startswith("node_") else None,
            node_attempt=3 if event_type.startswith("node_") else None,
        )

    events = _read_sse(
        client,
        "/v1/runs/run-event-types/events?after=4",
        expected_count=len(WORKER_EVENT_TYPES),
    )
    payloads = [event["data"] for event in events]

    assert [event["event"] for event in events] == list(WORKER_EVENT_TYPES)
    assert [event["id"] for event in events] == [str(payload["sequence"]) for payload in payloads]
    assert [payload["sequence"] for payload in payloads] == list(range(5, 5 + len(WORKER_EVENT_TYPES)))
    assert all(payload["run_id"] == "run-event-types" for payload in payloads)
    assert all(payload["worker_id"] == "worker-a" for payload in payloads)
    assert all(payload["execution_profile_id"] == "remote-dev" for payload in payloads)
    assert all(payload["timestamp"] for payload in payloads)
    assert all(payload["payload"] == {"source": "test"} for payload in payloads)
    node_payloads = [payload for payload in payloads if str(payload["event_type"]).startswith("node_")]
    assert all(payload["node_id"] == "node-a" for payload in node_payloads)
    assert all(payload["node_attempt"] == 3 for payload in node_payloads)


def test_worker_cleanup_failure_remains_visible_in_run_snapshot() -> None:
    client = TestClient(create_worker_app(token="test-token", policies={"cleanup_failure": True}))
    run_request = {
        "run_id": "run-cleanup-fails",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    cleanup = client.delete("/v1/runs/run-cleanup-fails", headers=AUTH)
    snapshot = client.get("/v1/runs/run-cleanup-fails", headers=AUTH)

    assert cleanup.status_code == 500
    assert cleanup.json()["error"]["code"] == "cleanup_failed"
    payload = snapshot.json()
    assert payload["status"] == "ready"
    assert payload["last_error"]["code"] == "cleanup_failed"
    assert payload["events"][-1]["event_type"] == "cleanup_failed"


def test_worker_orphan_cleanup_policy_closes_run_without_control_plane_delete() -> None:
    runtime = InProcessWorkerRuntime()
    client = TestClient(
        create_worker_app(
            token="test-token",
            runtime=runtime,
            policies={"orphan_cleanup": {"enabled": True, "ttl_seconds": 0}},
        )
    )
    run_request = {
        "run_id": "run-orphan-cleanup",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    admitted = client.post("/v1/runs", json=run_request, headers=AUTH)
    snapshot = client.get("/v1/runs/run-orphan-cleanup", headers=AUTH)

    assert admitted.status_code == 202
    payload = snapshot.json()
    assert payload["status"] == "closed"
    assert payload["active_node"] is None
    assert payload["last_error"] is None
    assert payload["orphan_cleanup"]["enabled"] is True
    assert payload["orphan_cleanup"]["ttl_seconds"] == 0
    assert payload["orphan_cleanup"]["status"] == "closed"
    assert payload["orphan_cleanup"]["last_control_plane_seen_at"]
    assert payload["orphan_cleanup"]["eligible_at"]
    assert payload["orphan_cleanup"]["last_attempted_at"]
    assert "runtime-run-orphan-cleanup" in runtime.cleaned
    assert [event["event_type"] for event in payload["events"]][-2:] == ["worker_log", "run_closed"]
    assert payload["events"][-2]["payload"]["reason"] == "orphaned_control_plane"
    assert payload["events"][-1]["payload"] == {"status": "closed", "reason": "orphaned_control_plane"}
    assert "node_result" not in [event["event_type"] for event in payload["events"]]
    assert "run_failed" not in [event["event_type"] for event in payload["events"]]


def test_worker_orphan_cleanup_failure_remains_visible_without_terminal_rewrite() -> None:
    client = TestClient(
        create_worker_app(
            token="test-token",
            policies={
                "orphan_cleanup": {"enabled": True, "ttl_seconds": 0},
                "cleanup_failure": True,
            },
        )
    )
    run_request = {
        "run_id": "run-orphan-cleanup-fails",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    snapshot = client.get("/v1/runs/run-orphan-cleanup-fails", headers=AUTH).json()
    repeated_snapshot = client.get("/v1/runs/run-orphan-cleanup-fails", headers=AUTH).json()

    assert snapshot["status"] == "ready"
    assert snapshot["last_error"]["code"] == "cleanup_failed"
    assert snapshot["orphan_cleanup"]["status"] == "failed"
    assert snapshot["orphan_cleanup"]["last_error"]["code"] == "cleanup_failed"
    assert [event["event_type"] for event in snapshot["events"]][-2:] == ["worker_log", "cleanup_failed"]
    assert snapshot["events"][-1]["payload"]["reason"] == "orphaned_control_plane"
    assert snapshot["events"][-1]["payload"]["error"]["code"] == "cleanup_failed"
    assert repeated_snapshot["last_sequence"] == snapshot["last_sequence"]
    assert "node_result" not in [event["event_type"] for event in snapshot["events"]]
    assert "run_failed" not in [event["event_type"] for event in snapshot["events"]]


def test_worker_validation_errors_use_structured_json_error_shape() -> None:
    client = TestClient(create_worker_app(token="test-token"))

    response = client.post("/v1/runs", json={"run_id": ""}, headers=AUTH)

    assert response.status_code == 422
    assert response.json()["error"]["code"] == "invalid_request"
    assert response.json()["error"]["message"] == "Request validation failed."
    assert response.json()["error"]["retryable"] is False
    assert response.json()["error"]["details"]["errors"]


def test_worker_unknown_route_uses_structured_json_error_shape() -> None:
    client = TestClient(create_worker_app(token="test-token"))

    response = client.get("/v1/missing", headers=AUTH)

    assert response.status_code == 404
    assert response.json() == {
        "error": {
            "code": "not_found",
            "message": "Worker API route was not found.",
            "retryable": False,
            "details": {},
        }
    }


def test_worker_node_request_uses_node_execution_id_for_idempotence_and_conflicts() -> None:
    client = TestClient(create_worker_app(token="test-token"))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-node-id", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202
    request = {"node_execution_id": "exec-1", "node_id": "n1", "attempt": 1, "payload": {"x": 1}}

    accepted = client.post("/v1/runs/run-node-id/nodes", json=request, headers=AUTH)
    repeated = client.post("/v1/runs/run-node-id/nodes", json=request, headers=AUTH)
    conflict = client.post(
        "/v1/runs/run-node-id/nodes",
        json={**request, "payload": {"x": 2}},
        headers=AUTH,
    )

    assert accepted.status_code == 202
    assert repeated.status_code == 202
    assert repeated.json() == accepted.json()
    assert accepted.json()["node_execution_id"] == "exec-1"
    assert conflict.status_code == 409
    assert conflict.json()["error"]["code"] == "conflict"


def test_worker_bridge_converts_process_messages_and_failure_outcomes_to_worker_events() -> None:
    runtime = InProcessWorkerRuntime(
        policies={
            "node_process_messages": [
                {"type": "progress", "payload": {"phase": "working"}},
                {"type": "log", "payload": {"message": "hello"}},
                {"type": "human_gate_request", "gate_id": "gate-1", "question": {"text": "Continue?"}},
                {"type": "child_run_request", "request_id": "child-req", "child_run_id": "child-1"},
                {"type": "child_status_request", "request_id": "status-req", "run_id": "child-1"},
                {
                    "type": "result",
                    "outcome": {"status": "fail", "failure_reason": "modeled fail", "context_updates": {}},
                    "context": {"context.changed": True},
                    "runtime_metadata": {"pid": 123},
                },
            ]
        }
    )
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a", runtime=runtime))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-bridge", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    response = client.post("/v1/runs/run-bridge/nodes", json={"node_id": "n1"}, headers=AUTH)
    _wait_for_snapshot_event(client, "run-bridge", "node_result")
    snapshot = client.get("/v1/runs/run-bridge", headers=AUTH).json()
    node_events = snapshot["events"][4:]

    assert response.status_code == 202
    assert [event["event_type"] for event in node_events] == [
        "node_started",
        "node_event",
        "worker_log",
        "human_gate_request",
        "child_run_request",
        "child_status_request",
        "node_result",
    ]
    assert node_events[-1]["payload"]["outcome"]["status"] == "fail"
    assert node_events[-1]["payload"]["runtime_metadata"] == {"pid": 123}
    assert all(event["node_id"] == "n1" for event in node_events)


def test_worker_bridge_converts_process_errors_to_node_failed_events() -> None:
    class FailingRuntime(InProcessWorkerRuntime):
        def run_node(self, handle, request: dict[str, Any], callbacks) -> dict[str, Any]:
            raise WorkerAPIError("invalid_process_message", "bad json", 502, details={"line": "{"})

    client = TestClient(create_worker_app(token="test-token", runtime=FailingRuntime()))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-node-failed", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    response = client.post("/v1/runs/run-node-failed/nodes", json={"node_id": "n1"}, headers=AUTH)
    _wait_for_snapshot_event(client, "run-node-failed", "node_failed")
    snapshot = client.get("/v1/runs/run-node-failed", headers=AUTH).json()

    assert response.status_code == 202
    assert snapshot["events"][-1]["event_type"] == "node_failed"
    assert snapshot["events"][-1]["payload"]["code"] == "invalid_process_message"
    assert snapshot["active_node"] is None


def test_worker_node_request_accepts_without_waiting_for_later_runtime_completion() -> None:
    class BlockingRuntime(InProcessWorkerRuntime):
        def __init__(self) -> None:
            super().__init__()
            self.started = threading.Event()
            self.release = threading.Event()

        def run_node(self, handle, request: dict[str, Any], callbacks) -> dict[str, Any]:
            self.started.set()
            assert self.release.wait(timeout=5)
            return super().run_node(handle, request, callbacks)

    runtime = BlockingRuntime()
    app = create_worker_app(token="test-token", runtime=runtime)
    server, server_thread, base_url = _start_test_server(app)

    try:
        with httpx.Client(base_url=base_url, timeout=5.0) as client:
            assert client.post(
                "/v1/runs",
                json={"run_id": "run-nonblocking", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
                headers=AUTH,
            ).status_code == 202

            accepted = client.post("/v1/runs/run-nonblocking/nodes", json={"node_id": "n1"}, headers=AUTH)
            assert accepted.status_code == 202
            assert runtime.started.wait(timeout=2)

            before_release = client.get("/v1/runs/run-nonblocking", headers=AUTH).json()
            assert [event["event_type"] for event in before_release["events"][-1:]] == ["node_started"]

            runtime.release.set()
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                snapshot = client.get("/v1/runs/run-nonblocking", headers=AUTH).json()
                if snapshot["events"][-1]["event_type"] == "node_result":
                    break
                time.sleep(0.01)
            else:
                raise AssertionError("node_result was not emitted after runtime completion")
    finally:
        runtime.release.set()
        _stop_test_server(server, server_thread)


def test_worker_callback_delivery_resumes_active_node_request() -> None:
    class CallbackRuntime(InProcessWorkerRuntime):
        def __init__(self) -> None:
            super().__init__()
            self.request_emitted = threading.Event()

        def run_node(self, handle, request: dict[str, Any], callbacks) -> dict[str, Any]:
            payload = {"type": "human_gate_request", "gate_id": "gate-1", "question": {"text": "Continue?"}}
            callbacks.emit_process_event("human_gate_request", {"gate_id": "gate-1", "question": payload["question"]})
            self.request_emitted.set()
            answer = callbacks.resolve_process_request("human_gate_request", payload)
            return {
                "type": "result",
                "outcome": {
                    "status": "success",
                    "context_updates": {"context.answer": answer["value"]},
                },
                "context": {},
                "runtime_metadata": {},
            }

    runtime = CallbackRuntime()
    client = TestClient(create_worker_app(token="test-token", runtime=runtime))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-callback-resume", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    accepted = client.post("/v1/runs/run-callback-resume/nodes", json={"node_id": "n1"}, headers=AUTH)
    assert accepted.status_code == 202
    assert runtime.request_emitted.wait(timeout=2)

    callback = client.post(
        "/v1/runs/run-callback-resume/human-gates/gate-1/answer",
        json={"payload": {"value": "YES"}},
        headers=AUTH,
    )
    result_event = _wait_for_snapshot_event(client, "run-callback-resume", "node_result")

    assert callback.status_code == 200
    assert result_event["payload"]["outcome"]["context_updates"] == {"context.answer": "YES"}


def test_worker_invalid_outcome_shape_is_emitted_as_node_failed() -> None:
    class InvalidOutcomeRuntime(InProcessWorkerRuntime):
        def run_node(self, handle, request: dict[str, Any], callbacks) -> dict[str, Any]:
            return {"type": "result", "outcome": {"status": "not-a-real-status", "context_updates": {}}}

    client = TestClient(create_worker_app(token="test-token", runtime=InvalidOutcomeRuntime()))
    assert client.post(
        "/v1/runs",
        json={"run_id": "run-invalid-outcome", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
        headers=AUTH,
    ).status_code == 202

    response = client.post("/v1/runs/run-invalid-outcome/nodes", json={"node_id": "n1"}, headers=AUTH)
    event = _wait_for_snapshot_event(client, "run-invalid-outcome", "node_failed")

    assert response.status_code == 202
    assert event["payload"]["code"] == "invalid_process_result"


def test_worker_rejects_second_node_while_one_execution_is_active() -> None:
    class BlockingRuntime(InProcessWorkerRuntime):
        def __init__(self) -> None:
            super().__init__()
            self.started = threading.Event()
            self.release = threading.Event()

        def run_node(self, handle, request: dict[str, Any], callbacks) -> dict[str, Any]:
            self.started.set()
            assert self.release.wait(timeout=5)
            return super().run_node(handle, request, callbacks)

    runtime = BlockingRuntime()
    app = create_worker_app(token="test-token", runtime=runtime)
    server, server_thread, base_url = _start_test_server(app)
    errors: list[BaseException] = []
    first_result: list[httpx.Response] = []

    try:
        with httpx.Client(base_url=base_url, timeout=5.0) as client:
            assert client.post(
                "/v1/runs",
                json={"run_id": "run-active", "execution_profile_id": "remote", "mapped_project_path": "/srv/p"},
                headers=AUTH,
            ).status_code == 202

            def submit_first() -> None:
                try:
                    first_result.append(client.post("/v1/runs/run-active/nodes", json={"node_id": "n1"}, headers=AUTH))
                except BaseException as exc:  # pragma: no cover - surfaced below.
                    errors.append(exc)

            thread = threading.Thread(target=submit_first)
            thread.start()
            assert runtime.started.wait(timeout=2)

            second = client.post("/v1/runs/run-active/nodes", json={"node_id": "n2"}, headers=AUTH)
            runtime.release.set()
            thread.join(timeout=2)

        assert second.status_code == 409
        assert second.json()["error"]["code"] == "active_node_exists"
        assert errors == []
        assert first_result[0].status_code == 202
    finally:
        runtime.release.set()
        _stop_test_server(server, server_thread)
