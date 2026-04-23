from __future__ import annotations

import asyncio
import json
from pathlib import Path
import time

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import spark.app as product_app
from attractor.api.token_usage import TokenUsageBreakdown, TokenUsageBucket, estimate_model_cost
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_completion as _wait_for_pipeline_completion,
)
from tests.support.flow_fixtures import seed_flow_fixture

def _seed_flow(name: str) -> None:
    seed_flow_fixture(product_app.get_settings().flows_dir, "minimal-valid.dot", as_name=name)


class _DisconnectImmediatelyRequest:
    async def is_disconnected(self) -> bool:
        return True


class _DisconnectAfterEventChecksRequest:
    def __init__(self, *, checks: int) -> None:
        self._checks = checks
        self._count = 0

    async def is_disconnected(self) -> bool:
        self._count += 1
        return self._count > self._checks


class _QueueOnlyRunListEventHub:
    def __init__(self, queued_events: list[dict] | None = None) -> None:
        self._queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=8)
        for event in queued_events or []:
            self._queue.put_nowait(dict(event))

    async def publish(self, event: dict) -> None:
        await self._queue.put(dict(event))

    def subscribe(self) -> asyncio.Queue[dict]:
        return self._queue

    def unsubscribe(self, queue: asyncio.Queue[dict]) -> None:
        assert queue is self._queue

    def reset(self) -> None:
        pass


def _decode_sse_event(chunk: str | bytes) -> dict:
    text = chunk.decode("utf-8") if isinstance(chunk, bytes) else chunk
    lines = [line for line in text.splitlines() if line.startswith("data: ")]
    assert lines
    return json.loads(lines[0].removeprefix("data: "))


async def _collect_runs_stream_events(
    *,
    request,
    count: int,
    project_path: str | None = None,
) -> tuple[dict[str, str], list[dict]]:
    response = await server.runs_events(request, project_path=project_path)
    iterator = response.body_iterator
    chunks: list[str | bytes] = []
    try:
        for _ in range(count):
            chunks.append(await anext(iterator))
    finally:
        await iterator.aclose()
    return dict(response.headers), [_decode_sse_event(chunk) for chunk in chunks]


async def _next_run_list_event(queue: asyncio.Queue[dict], *, timeout_seconds: float = 1.0) -> dict:
    return await asyncio.wait_for(queue.get(), timeout=timeout_seconds)


def test_runs_overview_endpoints_are_registered() -> None:
    seen: set[tuple[str, str]] = set()
    for route in server.attractor_app.routes:
        path = getattr(route, "path", None)
        methods = getattr(route, "methods", None)
        if not path or not methods:
            continue
        for method in methods:
            if method == "GET":
                seen.add((method, path))

    assert ("GET", "/runs") in seen
    assert ("GET", "/runs/events") in seen


def test_list_runs_includes_project_and_git_metadata_fields(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-project-metadata"

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project"),
            git_branch="main",
            git_commit="abc123",
            last_error="",
            token_usage=42,
        )
    )

    response = product_api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["project_path"] == str(tmp_path / "project")
    assert run_payload["git_branch"] == "main"
    assert run_payload["git_commit"] == "abc123"


def test_list_runs_includes_structured_usage_and_estimated_cost(
    product_api_client: TestClient,
    tmp_path: Path,
) -> None:
    run_id = "run-with-structured-usage"
    breakdown = TokenUsageBreakdown(
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
    )

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "work"),
            model="gpt-5.5",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project"),
            token_usage=36,
            token_usage_breakdown=breakdown,
            estimated_model_cost=estimate_model_cost(breakdown),
        )
    )

    response = product_api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()["runs"][0]
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
    assert payload["estimated_model_cost"]["status"] == "partial_unpriced"
    assert payload["estimated_model_cost"]["unpriced_models"] == ["gpt-5.3-codex-spark"]
    assert payload["estimated_model_cost"]["amount"] == pytest.approx(0.000332, rel=0, abs=1e-9)


def test_list_runs_includes_spec_and_plan_artifact_links_when_available_item_9_6_03(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-artifact-links"

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project"),
            git_branch="main",
            git_commit="abc123",
            spec_id="spec-project-1700000000",
            plan_id="plan-project-1700000000",
        )
    )

    response = product_api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["spec_id"] == "spec-project-1700000000"
    assert run_payload["plan_id"] == "plan-project-1700000000"


def test_list_runs_filters_durable_history_by_project_item_9_6_01(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-root",
            flow_name="Flow A",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "project-alpha"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project-alpha"),
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-child",
            flow_name="Flow B",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "project-alpha" / "nested"),
            model="test-model",
            started_at="2026-01-01T00:02:00Z",
            ended_at="2026-01-01T00:03:00Z",
            project_path="",
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-other-project",
            flow_name="Flow C",
            status="failed",
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "project-beta"),
            model="test-model",
            started_at="2026-01-01T00:04:00Z",
            ended_at="2026-01-01T00:05:00Z",
            project_path=str(tmp_path / "project-beta"),
        )
    )

    filtered_response = product_api_client.get(
        "/attractor/runs",
        params={"project_path": str(tmp_path / "project-alpha")},
    )
    assert filtered_response.status_code == 200
    filtered_payload = filtered_response.json()
    filtered_run_ids = {run["run_id"] for run in filtered_payload["runs"]}

    assert filtered_run_ids == {"run-in-project-root", "run-in-project-child"}


def test_list_runs_backfills_missing_timestamps_from_run_log_item_9_6_04(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-partial-timestamps"
    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "project"),
            model="test-model",
            started_at="",
            ended_at=None,
            project_path=str(tmp_path / "project"),
        )
    )
    run_root = server._run_root(run_id)
    (run_root / "run.log").write_text(
        "\n".join(
            [
                "[2026-01-01 00:10:00 UTC] Starting run",
                "[2026-01-01 00:10:30 UTC] Pipeline success",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    response = product_api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["started_at"] == "2026-01-01T00:10:00Z"
    assert run_payload["ended_at"] == "2026-01-01T00:10:30Z"


def test_list_runs_reconstructs_timestamp_ordering_from_run_logs_item_9_6_04(
    product_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    older_id = "run-older"
    newer_id = "run-newer"

    for run_id, start_ts, end_ts in [
        (older_id, "2026-01-01 00:00:00", "2026-01-01 00:00:30"),
        (newer_id, "2026-01-01 00:01:00", "2026-01-01 00:01:30"),
    ]:
        server._write_run_meta(
            server.RunRecord(
                run_id=run_id,
                flow_name="Flow",
                status="completed",
                outcome="success",
                outcome_reason_code=None,
                outcome_reason_message=None,
                working_directory=str(tmp_path / "project"),
                model="test-model",
                started_at="",
                ended_at=None,
                project_path=str(tmp_path / "project"),
            )
        )
        run_root = server._run_root(run_id)
        (run_root / "run.log").write_text(
            "\n".join(
                [
                    f"[{start_ts} UTC] Starting run",
                    f"[{end_ts} UTC] Pipeline success",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

    response = product_api_client.get("/attractor/runs")
    assert response.status_code == 200
    payload = response.json()
    run_ids = [run["run_id"] for run in payload["runs"]]

    assert run_ids == [newer_id, older_id]


def test_runs_events_snapshot_filters_durable_history_by_project_and_returns_run_record_payload(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    project_alpha = str(tmp_path / "project-alpha")
    project_beta = str(tmp_path / "project-beta")
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-root",
            flow_name="Flow A",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=project_alpha,
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=project_alpha,
            git_branch="main",
            git_commit="abc123",
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-child",
            flow_name="Flow B",
            status="completed",
            outcome="success",
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "project-alpha" / "nested"),
            model="test-model",
            started_at="2026-01-01T00:02:00Z",
            ended_at="2026-01-01T00:03:00Z",
            project_path="",
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-other-project",
            flow_name="Flow C",
            status="failed",
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=project_beta,
            model="test-model",
            started_at="2026-01-01T00:04:00Z",
            ended_at="2026-01-01T00:05:00Z",
            project_path=project_beta,
        )
    )

    headers, events = asyncio.run(
        _collect_runs_stream_events(
            request=_DisconnectImmediatelyRequest(),
            count=1,
            project_path=project_alpha,
        )
    )

    assert headers["content-type"].startswith("text/event-stream")
    assert headers["cache-control"] == "no-cache"
    assert headers["connection"] == "keep-alive"
    assert events == [
        {
            "type": "snapshot",
            "runs": [
                {
                    "run_id": "run-in-project-child",
                    "flow_name": "Flow B",
                    "status": "completed",
                    "outcome": "success",
                    "outcome_reason_code": None,
                    "outcome_reason_message": None,
                    "working_directory": str(tmp_path / "project-alpha" / "nested"),
                    "model": "test-model",
                    "started_at": "2026-01-01T00:02:00Z",
                    "ended_at": "2026-01-01T00:03:00Z",
                    "project_path": "",
                    "git_branch": None,
                    "git_commit": None,
                    "spec_id": None,
                    "plan_id": None,
                    "continued_from_run_id": None,
                    "continued_from_node": None,
                    "continued_from_flow_mode": None,
                    "continued_from_flow_name": None,
                    "parent_run_id": None,
                    "parent_node_id": None,
                    "root_run_id": None,
                    "child_invocation_index": None,
                    "last_error": "",
                    "token_usage": None,
                    "token_usage_breakdown": None,
                    "estimated_model_cost": None,
                },
                {
                    "run_id": "run-in-project-root",
                    "flow_name": "Flow A",
                    "status": "completed",
                    "outcome": "success",
                    "outcome_reason_code": None,
                    "outcome_reason_message": None,
                    "working_directory": project_alpha,
                    "model": "test-model",
                    "started_at": "2026-01-01T00:00:00Z",
                    "ended_at": "2026-01-01T00:01:00Z",
                    "project_path": project_alpha,
                    "git_branch": "main",
                    "git_commit": "abc123",
                    "spec_id": None,
                    "plan_id": None,
                    "continued_from_run_id": None,
                    "continued_from_node": None,
                    "continued_from_flow_mode": None,
                    "continued_from_flow_name": None,
                    "parent_run_id": None,
                    "parent_node_id": None,
                    "root_run_id": None,
                    "child_invocation_index": None,
                    "last_error": "",
                    "token_usage": None,
                    "token_usage_breakdown": None,
                    "estimated_model_cost": None,
                },
            ],
        }
    ]


def test_runs_events_filters_live_run_upserts_by_project_path(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    project_alpha = str(tmp_path / "project-alpha")
    project_beta = str(tmp_path / "project-beta")
    selected_record = server.RunRecord(
        run_id="run-alpha",
        flow_name="alpha.dot",
        status="running",
        outcome=None,
        outcome_reason_code=None,
        outcome_reason_message=None,
        working_directory=project_alpha,
        model="test-model",
        started_at="2026-01-01T00:00:00Z",
        project_path=project_alpha,
    )
    server._write_run_meta(selected_record)
    monkeypatch.setattr(
        server,
        "RUNS_EVENT_HUB",
        _QueueOnlyRunListEventHub(
            queued_events=[
                {
                    "type": "run_upsert",
                    "run": server.RunRecord(
                        run_id="run-beta",
                        flow_name="beta.dot",
                        status="running",
                        outcome=None,
                        outcome_reason_code=None,
                        outcome_reason_message=None,
                        working_directory=project_beta,
                        model="test-model",
                        started_at="2026-01-01T00:01:00Z",
                        project_path=project_beta,
                    ).to_dict(),
                },
                {
                    "type": "run_upsert",
                    "run": selected_record.to_dict(),
                },
            ]
        ),
    )

    _, events = asyncio.run(
        _collect_runs_stream_events(
            request=_DisconnectAfterEventChecksRequest(checks=2),
            count=2,
            project_path=project_alpha,
        )
    )

    assert events[0]["type"] == "snapshot"
    assert [run["run_id"] for run in events[0]["runs"]] == ["run-alpha"]
    assert events[1] == {
        "type": "run_upsert",
        "run": selected_record.to_dict(),
    }


def test_run_list_upsert_publishes_on_pipeline_start_metadata_patch_and_cancel_transition(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    hub = server.RunListEventHub()
    monkeypatch.setattr(server, "RUNS_EVENT_HUB", hub)
    queue = hub.subscribe()

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["run_id"])

    start_event = asyncio.run(_next_run_list_event(queue))
    assert start_event["type"] == "run_upsert"
    assert start_event["run"]["run_id"] == run_id
    assert start_event["run"]["status"] == "running"
    assert start_event["run"]["started_at"]

    metadata_response = attractor_api_client.patch(
        f"/pipelines/{run_id}/metadata",
        json={"spec_id": "spec-123", "plan_id": "plan-123"},
    )
    assert metadata_response.status_code == 200

    metadata_event = asyncio.run(_next_run_list_event(queue))
    assert metadata_event["type"] == "run_upsert"
    assert metadata_event["run"]["run_id"] == run_id
    assert metadata_event["run"]["status"] == "running"
    assert metadata_event["run"]["spec_id"] == "spec-123"
    assert metadata_event["run"]["plan_id"] == "plan-123"

    cancel_response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert cancel_response.status_code == 200
    assert cancel_response.json() == {"status": "cancel_requested", "pipeline_id": run_id}

    cancel_event = asyncio.run(_next_run_list_event(queue))
    assert cancel_event["type"] == "run_upsert"
    assert cancel_event["run"]["run_id"] == run_id
    assert cancel_event["run"]["status"] == "cancel_requested"
    assert cancel_event["run"]["last_error"] == "cancel_requested_by_user"


def test_run_list_upsert_publishes_on_pipeline_terminal_completion(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    hub = server.RunListEventHub()
    monkeypatch.setattr(server, "RUNS_EVENT_HUB", hub)
    queue = hub.subscribe()

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["run_id"])

    start_event = asyncio.run(_next_run_list_event(queue))
    assert start_event["type"] == "run_upsert"
    assert start_event["run"]["run_id"] == run_id
    assert start_event["run"]["status"] == "running"

    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id)
    assert final_payload["status"] == "completed"

    completion_event = asyncio.run(_next_run_list_event(queue, timeout_seconds=2.0))
    assert completion_event["type"] == "run_upsert"
    assert completion_event["run"]["run_id"] == run_id
    assert completion_event["run"]["status"] == "completed"
    assert completion_event["run"]["outcome"] == "success"
    assert completion_event["run"]["ended_at"] is not None


def test_run_list_upsert_includes_structured_usage_fields(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    breakdown = TokenUsageBreakdown(
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
    )
    run_id = "run-upsert-structured-usage"
    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="alpha.dot",
            status="running",
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=str(tmp_path / "work"),
            model="gpt-5.5",
            started_at="2026-01-01T00:00:00Z",
            project_path=str(tmp_path / "project"),
            token_usage=36,
            token_usage_breakdown=breakdown,
            estimated_model_cost=estimate_model_cost(breakdown),
        )
    )
    hub = server.RunListEventHub()
    monkeypatch.setattr(server, "RUNS_EVENT_HUB", hub)
    queue = hub.subscribe()

    asyncio.run(server._publish_run_list_upsert(run_id))

    event = asyncio.run(_next_run_list_event(queue))
    assert event["type"] == "run_upsert"
    assert event["run"]["token_usage"] == 36
    assert event["run"]["token_usage_breakdown"]["by_model"]["gpt-5.5"]["total_tokens"] == 24
    assert event["run"]["estimated_model_cost"]["status"] == "partial_unpriced"
    assert event["run"]["estimated_model_cost"]["unpriced_models"] == ["gpt-5.3-codex-spark"]


def test_run_list_upsert_publishes_when_startup_reconciles_orphaned_active_run(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    run_id = "run-orphaned-running"
    project_path = str(tmp_path / "project")
    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="orphaned.dot",
            status="running",
            outcome=None,
            outcome_reason_code=None,
            outcome_reason_message=None,
            working_directory=project_path,
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            project_path=project_path,
        )
    )
    hub = server.RunListEventHub()
    monkeypatch.setattr(server, "RUNS_EVENT_HUB", hub)
    queue = hub.subscribe()

    with TestClient(server.attractor_app) as client:
        response = client.get("/status")
        assert response.status_code == 200

        reconciled_event = asyncio.run(_next_run_list_event(queue))

    assert reconciled_event["type"] == "run_upsert"
    assert reconciled_event["run"]["run_id"] == run_id
    assert reconciled_event["run"]["status"] == "failed"
    assert "interrupted when the Attractor server stopped before completion" in reconciled_event["run"]["last_error"]
