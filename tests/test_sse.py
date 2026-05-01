from __future__ import annotations

from collections.abc import AsyncIterator
import json

from fastapi.testclient import TestClient
import pytest

from attractor.execution.worker_app import create_worker_app
import unified_llm.provider_utils as provider_utils


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


def test_parse_sse_events_preserves_boundaries_retry_comments_and_raw_payloads() -> None:
    payload = (
        ": keep-alive\n"
        "event: response.output_text.delta\n"
        "data: {\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}\n"
        "data: {\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}\n"
        "retry: 1500\n"
        "\n"
        "data: plain text\n"
        "\n"
    )

    events = list(provider_utils.parse_sse_events(payload))

    assert len(events) == 2
    first, second = events
    assert first.type == "response.output_text.delta"
    assert first.event == "response.output_text.delta"
    assert first.comment == "keep-alive"
    assert first.retry == 1500
    assert first.data == (
        "{\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}\n"
        "{\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}"
    )
    assert first.data_lines == (
        "{\"type\": \"response.output_text.delta\", \"delta\": \"Hel\"}",
        "{\"type\": \"response.output_text.delta\", \"delta\": \"lo\"}",
    )
    assert "response.output_text.delta" in first.raw

    assert second.type == "message"
    assert second.data == "plain text"
    assert second.comment is None
    assert second.retry is None
    assert second.raw == "data: plain text"


def test_worker_sse_events_include_id_event_and_required_metadata() -> None:
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a"))
    run_request = {
        "run_id": "run-sse-metadata",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    assert client.post(
        "/v1/runs/run-sse-metadata/nodes",
        json={"node_id": "implement", "attempt": 2, "payload": {"step": "execute"}},
        headers=AUTH,
    ).status_code == 202

    events = _read_sse(client, "/v1/runs/run-sse-metadata/events?after=4", expected_count=2)

    assert [event["id"] for event in events] == ["5", "6"]
    assert [event["event"] for event in events] == ["node_started", "node_result"]
    started = events[0]["data"]
    assert started["run_id"] == "run-sse-metadata"
    assert started["sequence"] == 5
    assert started["event_type"] == "node_started"
    assert started["worker_id"] == "worker-a"
    assert started["execution_profile_id"] == "remote-dev"
    assert started["node_id"] == "implement"
    assert started["node_attempt"] == 2
    assert started["timestamp"]
    assert started["payload"] == {"status": "started", "payload": {"step": "execute"}, "context": {}}


def test_worker_sse_replays_after_last_event_id_and_after_query() -> None:
    client = TestClient(create_worker_app(token="test-token", worker_id="worker-a"))
    run_request = {
        "run_id": "run-sse-replay",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    assert client.post("/v1/runs/run-sse-replay/nodes", json={"node_id": "n1", "attempt": 1}, headers=AUTH).status_code == 202
    assert client.post("/v1/runs/run-sse-replay/cancel", headers=AUTH).status_code == 200

    after_header = _read_sse(
        client,
        "/v1/runs/run-sse-replay/events",
        headers={**AUTH, "Last-Event-ID": "4"},
        expected_count=3,
    )
    after_query = _read_sse(client, "/v1/runs/run-sse-replay/events?after=4", expected_count=3)
    after_query_wins = _read_sse(
        client,
        "/v1/runs/run-sse-replay/events?after=5",
        headers={**AUTH, "Last-Event-ID": "1"},
        expected_count=2,
    )

    assert after_header == after_query
    assert [event["id"] for event in after_query] == ["5", "6", "7"]
    assert [event["event"] for event in after_query] == ["node_started", "node_result", "run_canceling"]
    assert [event["id"] for event in after_query_wins] == ["6", "7"]


def test_worker_sse_streams_require_bearer_auth_before_opening_body() -> None:
    client = TestClient(create_worker_app(token="test-token"))
    run_request = {
        "run_id": "run-sse-auth",
        "execution_profile_id": "remote-dev",
        "mapped_project_path": "/srv/projects/acme",
    }

    assert client.post("/v1/runs", json=run_request, headers=AUTH).status_code == 202
    response = client.get("/v1/runs/run-sse-auth/events")

    assert response.status_code == 401
    assert response.headers["content-type"].startswith("application/json")
    assert response.json()["error"]["code"] == "unauthorized"


def test_parse_sse_events_ignores_invalid_retry_values_and_keeps_late_event_type() -> None:
    payload = (
        "retry: invalid\n"
        "data: one\n"
        "event: custom.event\n"
        "\n"
    )

    events = list(provider_utils.parse_sse(payload))

    assert len(events) == 1
    event = events[0]
    assert event.type == "custom.event"
    assert event.data == "one"
    assert event.retry is None
    assert event.raw == "retry: invalid\ndata: one\nevent: custom.event"


@pytest.mark.asyncio
async def test_aiter_sse_events_preserves_boundaries_retry_comments_and_raw_payloads() -> None:
    async def source() -> AsyncIterator[str]:
        yield ": keep-alive\n"
        yield "event: response.output_text.delta\n"
        yield 'data: {"type": "response.output_text.delta", "delta": "Hel"}\n'
        yield 'data: {"type": "response.output_text.delta", "delta": "lo"}\n'
        yield "retry: 1500\n"
        yield "\n"
        yield "data: plain text\n"
        yield "\n"

    events = [event async for event in provider_utils.aiter_sse_events(source())]

    assert len(events) == 2
    first, second = events
    assert first.type == "response.output_text.delta"
    assert first.event == "response.output_text.delta"
    assert first.comment == "keep-alive"
    assert first.retry == 1500
    assert first.data == (
        '{"type": "response.output_text.delta", "delta": "Hel"}\n'
        '{"type": "response.output_text.delta", "delta": "lo"}'
    )
    assert first.data_lines == (
        '{"type": "response.output_text.delta", "delta": "Hel"}',
        '{"type": "response.output_text.delta", "delta": "lo"}',
    )
    assert "response.output_text.delta" in first.raw

    assert second.type == "message"
    assert second.data == "plain text"
    assert second.comment is None
    assert second.retry is None
    assert second.raw == "data: plain text"
