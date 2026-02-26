from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, List, Optional


class OutcomeStatus(str, Enum):
    SUCCESS = "success"
    RETRY = "retry"
    FAIL = "fail"
    PARTIAL_SUCCESS = "partial_success"


@dataclass
class Outcome:
    status: OutcomeStatus
    preferred_label: str = ""
    suggested_next_ids: List[str] = field(default_factory=list)
    context_updates: Dict[str, str] = field(default_factory=dict)
    failure_reason: str = ""
    notes: str = ""
    retryable: Optional[bool] = None
