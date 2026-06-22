from __future__ import annotations

import queue
import threading

import attractor.api.server as server
from attractor.interviewer import Question, QuestionOption, QuestionType


def test_human_gate_broker_waits_for_explicit_answer() -> None:
    broker = server.HumanGateBroker()
    question = Question(
        text="Choose an option",
        type=QuestionType.MULTIPLE_CHOICE,
        options=[QuestionOption(label="Fix", value="fix", key="F")],
        stage="gate",
    )
    emitted_events: queue.Queue[dict] = queue.Queue()
    answers: queue.Queue[list[str]] = queue.Queue()

    def _request_gate() -> None:
        answer = broker.request(
            question=question,
            run_id="run-1",
            node_id="gate",
            flow_name="flow",
            emit=emitted_events.put,
        )
        answers.put(answer.selected_values)

    request_thread = threading.Thread(target=_request_gate)
    request_thread.start()

    gate_event = _next_event(emitted_events, "human_gate")
    assert gate_event["prompt"] == "Choose an option"
    assert gate_event["options"] == [{"label": "Fix", "value": "fix"}]
    assert broker.answer("run-1", str(gate_event["question_id"]), "fix") is True

    request_thread.join(timeout=1)
    assert request_thread.is_alive() is False
    assert answers.get_nowait() == ["fix"]


def test_human_gate_broker_rejects_answers_for_wrong_run() -> None:
    broker = server.HumanGateBroker()
    question = Question(
        text="Choose an option",
        type=QuestionType.MULTIPLE_CHOICE,
        options=[QuestionOption(label="Fix", value="fix", key="F")],
        stage="gate",
    )
    emitted_events: queue.Queue[dict] = queue.Queue()

    request_thread = threading.Thread(
        target=lambda: broker.request(
            question=question,
            run_id="run-1",
            node_id="gate",
            flow_name="flow",
            emit=emitted_events.put,
        ),
    )
    request_thread.start()

    gate_event = _next_event(emitted_events, "human_gate")
    assert broker.answer("other-run", str(gate_event["question_id"]), "fix") is False
    assert broker.answer("run-1", str(gate_event["question_id"]), "fix") is True

    request_thread.join(timeout=1)
    assert request_thread.is_alive() is False


def _next_event(events: queue.Queue[dict], event_type: str) -> dict:
    while True:
        event = events.get(timeout=1)
        if event.get("type") == event_type:
            return event
