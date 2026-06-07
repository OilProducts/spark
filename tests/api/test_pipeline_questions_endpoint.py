from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def test_list_pipeline_questions_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = attractor_api_client.get("/pipelines/missing-run/questions")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_list_pipeline_questions_returns_only_pending_questions_for_requested_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = str(_start_pipeline(attractor_api_client, tmp_path / "work")["pipeline_id"])

    broker = server.HumanGateBroker()
    with broker._lock:
        broker._pending["q-1"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": run_id,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Approve plan?",
            "options": [{"label": "Approve", "value": "approve"}],
        }
        broker._pending["q-2"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": "other-run",
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Ignore me",
            "options": [{"label": "Nope", "value": "nope"}],
        }
    monkeypatch.setattr(server, "HUMAN_BROKER", broker)

    response = attractor_api_client.get(f"/pipelines/{run_id}/questions")

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "questions": [
            {
                "question_id": "q-1",
                "run_id": run_id,
                "node_id": "gate",
                "flow_name": "Flow",
                "prompt": "Approve plan?",
                "options": [{"label": "Approve", "value": "approve"}],
            }
        ]
    }


def test_list_pipeline_questions_excludes_answered_questions_for_requested_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = str(_start_pipeline(attractor_api_client, tmp_path / "work")["pipeline_id"])

    broker = server.HumanGateBroker()
    with broker._lock:
        broker._pending["q-pending"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": run_id,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Approve plan?",
            "options": [{"label": "Approve", "value": "approve"}],
        }
        broker._pending["q-answered"] = {
            "event": server.threading.Event(),
            "answer": "approve",
            "run_id": run_id,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Already answered",
            "options": [{"label": "Approve", "value": "approve"}],
        }
    monkeypatch.setattr(server, "HUMAN_BROKER", broker)

    response = attractor_api_client.get(f"/pipelines/{run_id}/questions")

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "questions": [
            {
                "question_id": "q-pending",
                "run_id": run_id,
                "node_id": "gate",
                "flow_name": "Flow",
                "prompt": "Approve plan?",
                "options": [{"label": "Approve", "value": "approve"}],
            }
        ]
    }


def test_submit_pipeline_answer_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server, "HUMAN_BROKER", server.HumanGateBroker())

    response = attractor_api_client.post(
        "/pipelines/missing-run/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_submit_pipeline_answer_accepts_pending_question_for_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = str(_start_pipeline(attractor_api_client, tmp_path / "work")["pipeline_id"])

    broker = server.HumanGateBroker()
    with broker._lock:
        broker._pending["q-1"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": run_id,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Approve plan?",
            "options": [{"label": "Approve", "value": "approve"}],
        }
    monkeypatch.setattr(server, "HUMAN_BROKER", broker)

    response = attractor_api_client.post(
        f"/pipelines/{run_id}/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload == {"status": "accepted", "pipeline_id": run_id, "question_id": "q-1"}
    assert broker._pending["q-1"]["answer"] == "approve"
    assert broker._pending["q-1"]["event"].is_set() is True


def test_submit_pipeline_answer_publishes_targeted_live_answer_update(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    run_id = "run-answer-live-update"
    server._record_run_start(
        run_id,
        flow_name="Flow",
        working_directory=str(tmp_path / "work"),
        model="test-model",
    )
    broker = server.HumanGateBroker()
    with broker._lock:
        broker._pending["q-1"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": run_id,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Approve plan?",
            "options": [{"label": "Approve", "value": "approve"}],
        }
    hub = server.PipelineEventHub()
    queue = hub.subscribe(run_id)
    monkeypatch.setattr(server, "HUMAN_BROKER", broker)
    monkeypatch.setattr(server, "EVENT_HUB", hub)

    response = attractor_api_client.post(
        f"/pipelines/{run_id}/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 200
    event = asyncio.run(asyncio.wait_for(queue.get(), timeout=1.0))
    assert event["type"] == "InterviewCompleted"
    assert event["run_id"] == run_id
    assert event["question_id"] == "q-1"
    assert event["node_id"] == "gate"
    assert event["answer"] == "approve"
    assert event["outcome_provenance"] == "accepted"
    assert event["sequence"] == 1


def test_submit_pipeline_answer_rejects_question_owned_by_other_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_a = str(_start_pipeline(attractor_api_client, tmp_path / "work-a")["pipeline_id"])
    run_b = str(_start_pipeline(attractor_api_client, tmp_path / "work-b")["pipeline_id"])

    broker = server.HumanGateBroker()
    with broker._lock:
        broker._pending["q-1"] = {
            "event": server.threading.Event(),
            "answer": None,
            "run_id": run_a,
            "node_id": "gate",
            "flow_name": "Flow",
            "prompt": "Approve plan?",
            "options": [{"label": "Approve", "value": "approve"}],
        }
    monkeypatch.setattr(server, "HUMAN_BROKER", broker)

    response = attractor_api_client.post(
        f"/pipelines/{run_b}/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown question for pipeline"
