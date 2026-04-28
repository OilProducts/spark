from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from fastapi.testclient import TestClient


SIMPLE_FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""

TERMINAL_PIPELINE_STATUSES = {
    "completed",
    "failed",
    "validation_error",
    "paused",
    "canceled",
}


def close_task_immediately(coro: Any) -> object:
    coro.close()

    class _DummyTask:
        pass

    return _DummyTask()


def start_pipeline(
    api_client: TestClient,
    working_directory: Path,
    *,
    flow_content: str = SIMPLE_FLOW,
    backend: str = "codex-app-server",
    launch_context: dict[str, object] | None = None,
    model: str | None = None,
    llm_profile: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "flow_content": flow_content,
        "working_directory": str(working_directory),
        "backend": backend,
        "launch_context": launch_context,
    }
    if model is not None:
        payload["model"] = model
    if llm_profile is not None:
        payload["llm_profile"] = llm_profile
    response = api_client.post(
        "/pipelines",
        json=payload,
    )
    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    return payload


def wait_for_pipeline_terminal_status(
    api_client: TestClient,
    pipeline_id: str,
    *,
    attempts: int = 400,
    interval_seconds: float = 0.01,
) -> str:
    for _ in range(attempts):
        response = api_client.get(f"/pipelines/{pipeline_id}")
        assert response.status_code == 200
        status = str(response.json()["status"])
        if status in TERMINAL_PIPELINE_STATUSES:
            return status
        time.sleep(interval_seconds)
    raise AssertionError("timed out waiting for pipeline completion")


def wait_for_pipeline_completion(
    api_client: TestClient,
    pipeline_id: str,
    *,
    attempts: int = 400,
    interval_seconds: float = 0.01,
) -> dict[str, Any]:
    for _ in range(attempts):
        response = api_client.get(f"/pipelines/{pipeline_id}")
        assert response.status_code == 200
        payload = response.json()
        if str(payload["status"]) in TERMINAL_PIPELINE_STATUSES:
            return payload
        time.sleep(interval_seconds)
    raise AssertionError("timed out waiting for pipeline completion")
