from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import List


class QuestionType(str, Enum):
    SINGLE_SELECT = "SINGLE_SELECT"
    MULTI_SELECT = "MULTI_SELECT"
    FREE_TEXT = "FREE_TEXT"
    CONFIRM = "CONFIRM"


@dataclass
class QuestionOption:
    label: str
    value: str


@dataclass
class Question:
    title: str
    prompt: str
    question_type: QuestionType
    options: List[QuestionOption] = field(default_factory=list)
    metadata: dict = field(default_factory=dict)


@dataclass
class Answer:
    selected_values: List[str] = field(default_factory=list)
    text: str = ""
