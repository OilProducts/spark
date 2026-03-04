from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server

FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""


def _close_task_immediately(coro):
    coro.close()

    class _DummyTask:
        pass

    return _DummyTask()


def _start_pipeline(api_client: TestClient, working_directory: Path) -> str:
    response = api_client.post(
        "/pipelines",
        json={
            "flow_content": FLOW,
            "working_directory": str(working_directory),
            "backend": "codex",
        },
    )
    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    return str(payload["pipeline_id"])


def test_list_pipeline_questions_returns_404_for_unknown_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    response = api_client.get("/pipelines/missing-run/questions")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_list_pipeline_questions_returns_only_pending_questions_for_requested_run(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = _start_pipeline(api_client, tmp_path / "work")

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

    response = api_client.get(f"/pipelines/{run_id}/questions")

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
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = _start_pipeline(api_client, tmp_path / "work")

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

    response = api_client.get(f"/pipelines/{run_id}/questions")

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
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server, "HUMAN_BROKER", server.HumanGateBroker())

    response = api_client.post(
        "/pipelines/missing-run/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_submit_pipeline_answer_accepts_pending_question_for_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_id = _start_pipeline(api_client, tmp_path / "work")

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

    response = api_client.post(
        f"/pipelines/{run_id}/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload == {"status": "accepted", "pipeline_id": run_id, "question_id": "q-1"}
    assert broker._pending["q-1"]["answer"] == "approve"
    assert broker._pending["q-1"]["event"].is_set() is True


def test_submit_pipeline_answer_rejects_question_owned_by_other_pipeline(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    run_a = _start_pipeline(api_client, tmp_path / "work-a")
    run_b = _start_pipeline(api_client, tmp_path / "work-b")

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

    response = api_client.post(
        f"/pipelines/{run_b}/questions/q-1/answer",
        json={"selected_value": "approve"},
    )

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown question for pipeline"
