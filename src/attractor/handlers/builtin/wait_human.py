from __future__ import annotations

from dataclasses import dataclass
import time

from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.interviewer import Answer, AnswerValue, Interviewer, Question, QuestionOption, QuestionType

from ..base import HandlerRuntime


@dataclass(frozen=True)
class _Choice:
    key: str
    label: str
    target: str


class WaitHumanHandler:
    def __init__(self, interviewer: Interviewer):
        self.interviewer = interviewer

    def execute(self, runtime: HandlerRuntime) -> Outcome:
        if not runtime.outgoing_edges:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No outgoing edges for human gate")

        choices: list[_Choice] = []
        options = []
        for edge in runtime.outgoing_edges:
            label_attr = edge.attrs.get("label")
            label = edge.target
            if label_attr is not None:
                raw_label = str(label_attr.value)
                if raw_label.strip():
                    label = raw_label
            key = _parse_accelerator_key(label)
            choices.append(_Choice(key=key, label=label, target=edge.target))
            options.append(QuestionOption(label=label, value=label, key=key))

        question = Question(
            text=runtime.prompt or "Choose next route",
            type=QuestionType.MULTIPLE_CHOICE,
            options=options,
            stage=runtime.node_id,
            metadata={"node_id": runtime.node_id},
        )

        started_at = time.perf_counter()
        runtime.emit("InterviewStarted", question=question.text, stage=runtime.node_id)
        answer = self.interviewer.ask(question)
        duration = time.perf_counter() - started_at
        if answer.value == AnswerValue.SKIPPED.value:
            runtime.emit(
                "InterviewCompleted",
                question=question.text,
                answer=answer.value,
                duration=duration,
                outcome_provenance="skipped",
            )
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="human skipped interaction")

        selected = _select_choice(answer, choices)
        emitted_answer = selected.label if selected else answer.value
        runtime.emit(
            "InterviewCompleted",
            question=question.text,
            answer=emitted_answer,
            duration=duration,
            outcome_provenance="accepted" if selected is not None else "skipped",
        )
        if selected is None:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="human skipped interaction")

        return Outcome(
            status=OutcomeStatus.SUCCESS,
            preferred_label=selected.label,
            suggested_next_ids=[selected.target],
            context_updates={
                "human.gate.selected": selected.key,
                "human.gate.label": selected.label,
            },
            notes="human selection applied",
        )


def _select_choice(answer: Answer, choices: list[_Choice]) -> _Choice | None:
    tokens = [value.strip() for value in answer.selected_values if value and value.strip()]
    if answer.selected_option is not None:
        if answer.selected_option.value and answer.selected_option.value.strip():
            tokens.append(answer.selected_option.value.strip())
        if answer.selected_option.label and answer.selected_option.label.strip():
            tokens.append(answer.selected_option.label.strip())
        if answer.selected_option.key and answer.selected_option.key.strip():
            tokens.append(answer.selected_option.key.strip())
    if answer.text and answer.text.strip():
        tokens.append(answer.text.strip())

    if not tokens:
        return None

    for token in tokens:
        normalized_token = token.lower()
        for choice in choices:
            if choice.target == token:
                return choice
            if choice.label == token:
                return choice
            if _normalize_label(choice.label) == _normalize_label(token):
                return choice
            if choice.key and choice.key.upper() == token.upper():
                return choice
            if choice.target.lower() == normalized_token:
                return choice

    return None


def _normalize_label(label: str) -> str:
    text = (label or "").strip().lower()
    if text.startswith("[") and "]" in text:
        text = text[text.find("]") + 1 :].strip()
    if len(text) >= 2 and text[0].isalnum() and text[1] == ")":
        text = text[2:].strip()
    if len(text) >= 3 and text[0].isalnum():
        idx = 1
        while idx < len(text) and text[idx] == " ":
            idx += 1
        if idx < len(text) and text[idx] == "-":
            text = text[idx + 1 :].strip()
    return text


def _parse_accelerator_key(label: str) -> str:
    text = (label or "").strip()
    if not text:
        return ""

    if text.startswith("[") and "]" in text:
        inside = text[1 : text.find("]")].strip()
        if inside:
            return inside[0].upper()

    if len(text) >= 2 and text[0].isalnum() and text[1] == ")":
        return text[0].upper()

    if len(text) >= 3 and text[0].isalnum():
        idx = 1
        while idx < len(text) and text[idx] == " ":
            idx += 1
        if idx < len(text) and text[idx] == "-":
            return text[0].upper()

    return text[0].upper()
