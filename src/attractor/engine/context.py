from __future__ import annotations

from dataclasses import dataclass, field
import copy
from contextlib import contextmanager
import threading
from typing import Any, Dict, List


class ReadWriteLock:
    def __init__(self) -> None:
        self._condition = threading.Condition(threading.Lock())
        self._readers = 0
        self._writer_active = False
        self._waiting_writers = 0

    @contextmanager
    def read_lock(self):
        with self._condition:
            while self._writer_active or self._waiting_writers > 0:
                self._condition.wait()
            self._readers += 1
        try:
            yield
        finally:
            with self._condition:
                self._readers -= 1
                if self._readers == 0:
                    self._condition.notify_all()

    @contextmanager
    def write_lock(self):
        with self._condition:
            self._waiting_writers += 1
            while self._writer_active or self._readers > 0:
                self._condition.wait()
            self._waiting_writers -= 1
            self._writer_active = True
        try:
            yield
        finally:
            with self._condition:
                self._writer_active = False
                self._condition.notify_all()


@dataclass
class Context:
    values: Dict[str, Any] = field(default_factory=dict)
    logs: List[str] = field(default_factory=list)
    lock: ReadWriteLock = field(default_factory=ReadWriteLock)

    def set(self, key: str, value: Any) -> None:
        _validate_context_key(key)
        with self.lock.write_lock():
            if value is None:
                self.values.pop(key, None)
            else:
                self.values[key] = value

    def get(self, key: str, default: Any = None) -> Any:
        with self.lock.read_lock():
            return self.values.get(key, default)

    def get_string(self, key: str, default: str = "") -> str:
        value = self.get(key, None)
        if value is None:
            return default
        return _to_string(value)

    def append_log(self, entry: str) -> None:
        with self.lock.write_lock():
            self.logs.append(str(entry))

    def snapshot(self) -> Dict[str, Any]:
        with self.lock.read_lock():
            return copy.deepcopy(self.values)

    def apply_updates(self, updates: Dict[str, Any]) -> None:
        for key in updates.keys():
            _validate_context_key(key)
        with self.lock.write_lock():
            for key, value in updates.items():
                if value is None:
                    self.values.pop(key, None)
                else:
                    self.values[key] = copy.deepcopy(value)

    def merge_updates(self, updates: Dict[str, Any]) -> None:
        self.apply_updates(updates)

    def clone(self) -> "Context":
        with self.lock.read_lock():
            return Context(values=copy.deepcopy(self.values), logs=copy.deepcopy(self.logs))

    def get_context_path(self, path: str) -> str:
        # path is expected to be the suffix after "context."
        if path == "":
            return ""

        with self.lock.read_lock():
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


_ALLOWED_NAMESPACED_PREFIXES: tuple[str, ...] = (
    "context.",
    "graph.",
    "internal.",
    "parallel.",
    "stack.",
    "human.gate.",
    "work.",
    "_attractor.",
)


def _validate_context_key(key: str) -> None:
    if "." not in key:
        return
    for prefix in _ALLOWED_NAMESPACED_PREFIXES:
        if key.startswith(prefix):
            return
    raise ValueError(f"Unsupported context key namespace: {key}")
