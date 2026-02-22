from __future__ import annotations

from collections import deque
from typing import Callable, Deque, Iterable

from .base import Interviewer
from .models import Answer, Question, QuestionType


class AutoApproveInterviewer(Interviewer):
    def ask(self, question: Question) -> Answer:
        if question.question_type == QuestionType.CONFIRM:
            return Answer(selected_values=["yes"])
        if question.options:
            return Answer(selected_values=[question.options[0].value])
        return Answer()


class ConsoleInterviewer(Interviewer):
    def ask(self, question: Question) -> Answer:
        print(question.title)
        print(question.prompt)
        if question.options:
            for idx, option in enumerate(question.options, 1):
                print(f"{idx}. {option.label}")

        raw = input("> ").strip()

        if question.question_type == QuestionType.FREE_TEXT:
            return Answer(text=raw)

        if question.question_type == QuestionType.MULTI_SELECT:
            picks = []
            if raw:
                for token in raw.split(","):
                    token = token.strip()
                    if token.isdigit():
                        idx = int(token)
                        if 1 <= idx <= len(question.options):
                            picks.append(question.options[idx - 1].value)
            return Answer(selected_values=picks)

        if question.options and raw.isdigit():
            idx = int(raw)
            if 1 <= idx <= len(question.options):
                return Answer(selected_values=[question.options[idx - 1].value])

        return Answer(text=raw)


class CallbackInterviewer(Interviewer):
    def __init__(self, callback: Callable[[Question], Answer]):
        self.callback = callback

    def ask(self, question: Question) -> Answer:
        return self.callback(question)


class QueueInterviewer(Interviewer):
    def __init__(self, answers: Iterable[Answer]):
        self._answers: Deque[Answer] = deque(answers)

    def ask(self, question: Question) -> Answer:
        if not self._answers:
            return Answer()
        return self._answers.popleft()
