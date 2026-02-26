from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional


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
    context_updates: Dict[str, Any] = field(default_factory=dict)
    failure_reason: str = ""
    notes: str = ""
    retryable: Optional[bool] = None

    def to_payload(self) -> Dict[str, Any]:
        return {
            "status": self.status.value,
            "preferred_label": self.preferred_label,
            "suggested_next_ids": list(self.suggested_next_ids),
            "context_updates": dict(self.context_updates),
            "notes": self.notes,
            "failure_reason": self.failure_reason,
        }
