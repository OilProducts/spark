from __future__ import annotations

from abc import ABC, abstractmethod

from .models import Answer, Question


class Interviewer(ABC):
    @abstractmethod
    def ask(self, question: Question) -> Answer:
        raise NotImplementedError
