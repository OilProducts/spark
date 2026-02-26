from __future__ import annotations

import attractor.api.server as server
from attractor.interviewer import Answer, AnswerValue, Question, QuestionOption, QuestionType


class _TimedOutEvent:
    def __init__(self) -> None:
        self.timeout = None

    def wait(self, timeout=None) -> bool:
        self.timeout = timeout
        return False

    def set(self) -> None:
        return None


def test_human_gate_broker_applies_question_default_when_wait_times_out(monkeypatch):
    created: dict[str, _TimedOutEvent] = {}

    def _event_factory() -> _TimedOutEvent:
        event = _TimedOutEvent()
        created["event"] = event
        return event

    monkeypatch.setattr(server.threading, "Event", _event_factory)

    broker = server.HumanGateBroker()
    default_answer = Answer(selected_values=["fix"])
    question = Question(
        text="Choose an option",
        type=QuestionType.MULTIPLE_CHOICE,
        options=[QuestionOption(label="Fix", value="fix", key="F")],
        default=default_answer,
        timeout_seconds=0.25,
        stage="gate",
    )

    answer = broker.request(
        question=question,
        run_id="run-1",
        node_id="gate",
        flow_name="flow",
        emit=lambda _event: None,
    )

    assert answer.selected_values == ["fix"]
    assert answer.value == "fix"
    assert created["event"].timeout == 0.25


def test_human_gate_broker_returns_timeout_when_wait_times_out_without_default(monkeypatch):
    created: dict[str, _TimedOutEvent] = {}

    def _event_factory() -> _TimedOutEvent:
        event = _TimedOutEvent()
        created["event"] = event
        return event

    monkeypatch.setattr(server.threading, "Event", _event_factory)

    broker = server.HumanGateBroker()
    question = Question(
        text="Choose an option",
        type=QuestionType.MULTIPLE_CHOICE,
        options=[QuestionOption(label="Fix", value="fix", key="F")],
        timeout_seconds=0.5,
        stage="gate",
    )

    answer = broker.request(
        question=question,
        run_id="run-1",
        node_id="gate",
        flow_name="flow",
        emit=lambda _event: None,
    )

    assert answer.value == AnswerValue.TIMEOUT.value
    assert answer.selected_values == [AnswerValue.TIMEOUT.value]
    assert created["event"].timeout == 0.5
