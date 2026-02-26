from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Dict, List, Optional


def _utc_timestamp() -> str:
    return datetime.now(timezone.utc).isoformat()


@dataclass
class Checkpoint:
    current_node: str
    completed_nodes: List[str] = field(default_factory=list)
    context: Dict[str, object] = field(default_factory=dict)
    retry_counts: Dict[str, int] = field(default_factory=dict)
    logs: List[str] = field(default_factory=list)
    timestamp: str = field(default_factory=_utc_timestamp)

    def to_dict(self) -> Dict[str, object]:
        return {
            "timestamp": self.timestamp,
            "current_node": self.current_node,
            "completed_nodes": list(self.completed_nodes),
            "context": dict(self.context),
            "retry_counts": dict(self.retry_counts),
            "logs": list(self.logs),
        }

    @classmethod
    def from_dict(cls, data: Dict[str, object]) -> "Checkpoint":
        return cls(
            timestamp=str(data.get("timestamp", _utc_timestamp())),
            current_node=str(data.get("current_node", "")),
            completed_nodes=[str(v) for v in data.get("completed_nodes", [])],
            context=dict(data.get("context", {})),
            retry_counts={str(k): int(v) for k, v in dict(data.get("retry_counts", {})).items()},
            logs=[str(v) for v in data.get("logs", [])],
        )


def save_checkpoint(path: Path, checkpoint: Checkpoint) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(checkpoint.to_dict(), f, indent=2, sort_keys=True)


def load_checkpoint(path: Path) -> Optional[Checkpoint]:
    if not path.exists():
        return None
    with path.open("r", encoding="utf-8") as f:
        data = json.load(f)
    return Checkpoint.from_dict(data)
