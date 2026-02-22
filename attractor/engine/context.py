from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict


@dataclass
class Context:
    values: Dict[str, Any] = field(default_factory=dict)

    def set(self, key: str, value: Any) -> None:
        self.values[key] = value

    def get(self, key: str, default: Any = "") -> Any:
        return self.values.get(key, default)

    def merge_updates(self, updates: Dict[str, Any]) -> None:
        for key, value in updates.items():
            self.values[key] = value

    def get_context_path(self, path: str) -> str:
        # path is expected to be the suffix after "context."
        if path == "":
            return ""

        # Flat key fallback first for keys that themselves contain dots.
        if path in self.values:
            return _to_string(self.values[path])

        cur: Any = self.values
        for part in path.split("."):
            if isinstance(cur, dict) and part in cur:
                cur = cur[part]
            else:
                return ""
        return _to_string(cur)


def _to_string(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)
