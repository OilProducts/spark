from __future__ import annotations

from dataclasses import dataclass

from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.interviewer import Answer, Interviewer, Question, QuestionOption, QuestionType

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
            title=f"Human Gate: {runtime.node_id}",
            prompt=runtime.prompt or "Choose next route",
            question_type=QuestionType.SINGLE_SELECT,
            options=options,
            metadata={"node_id": runtime.node_id},
        )

        answer = self.interviewer.ask(question)
        if _is_timeout(answer):
            default_choice = _default_choice(runtime.node_attrs, choices)
            if default_choice is None:
                return Outcome(status=OutcomeStatus.RETRY, failure_reason="human gate timeout, no default")
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                preferred_label=default_choice.label,
                notes="human selection applied",
            )

        selected = _select_choice(answer, choices)
        if selected is None:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="human skipped interaction")

        return Outcome(status=OutcomeStatus.SUCCESS, preferred_label=selected.label, notes="human selection applied")


def _is_timeout(answer: Answer) -> bool:
    if any(value and value.strip() for value in answer.selected_values):
        return False
    if answer.text and answer.text.strip():
        return False
    return True


def _default_choice(node_attrs, choices: list[_Choice]) -> _Choice | None:
    attr = node_attrs.get("human.default_choice")
    if not attr:
        return None

    default_target = str(attr.value).strip()
    if not default_target:
        return None

    for choice in choices:
        if choice.target == default_target:
            return choice
    for choice in choices:
        if choice.target.lower() == default_target.lower():
            return choice
    return None


def _select_choice(answer: Answer, choices: list[_Choice]) -> _Choice | None:
    tokens = [value.strip() for value in answer.selected_values if value and value.strip()]
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

    return choices[0] if choices else None


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
