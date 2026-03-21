from __future__ import annotations

from contextlib import contextmanager
import signal
from dataclasses import dataclass, field
from pathlib import Path
import threading
from typing import Any, Callable

from attractor.dsl.models import Duration
from attractor.dsl.models import DotGraph
from attractor.engine.artifacts import ArtifactStore
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus

from .base import HandlerRuntime
from .registry import HandlerRegistry


BUILTIN_HANDLER_TYPES = {
    "start",
    "exit",
    "codergen",
    "wait.human",
    "conditional",
    "parallel",
    "parallel.fan_in",
    "tool",
    "stack.manager_loop",
}


@dataclass
class HandlerRunner:
    graph: DotGraph
    registry: HandlerRegistry
    logs_root: Path | None = None
    artifact_store: ArtifactStore | None = None
    _concurrency_lock: threading.Lock = field(default_factory=threading.Lock, init=False, repr=False)
    _active_calls: int = field(default=0, init=False, repr=False)
    _concurrency_overrides: int = field(default=0, init=False, repr=False)
    _custom_handler_locks_guard: threading.Lock = field(default_factory=threading.Lock, init=False, repr=False)
    _custom_handler_locks: dict[int, threading.Lock] = field(default_factory=dict, init=False, repr=False)

    def __post_init__(self) -> None:
        if self.logs_root is not None:
            self.logs_root = Path(self.logs_root)
        self._sync_artifact_store()

    def __call__(self, node_id: str, prompt: str, context: Context) -> Outcome | None:
        return self._run(node_id, prompt, context, emit_event=None)

    def run_with_events(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        emit_event: Callable[..., None] | None,
    ) -> Outcome | None:
        return self._run(node_id, prompt, context, emit_event=emit_event)

    def _run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        emit_event: Callable[..., None] | None,
    ) -> Outcome | None:
        entered = False
        try:
            self._enter_call()
            entered = True
            node = self.graph.nodes[node_id]
            outgoing = [edge for edge in self.graph.edges if edge.source == node_id]
            handler_type = self.registry.resolve_handler_type(node)
            handler = self.registry.get(handler_type)
            runtime = HandlerRuntime(
                node_id=node_id,
                node=node,
                prompt=prompt,
                node_attrs=node.attrs,
                outgoing_edges=outgoing,
                context=context,
                graph=self.graph,
                logs_root=self.logs_root,
                artifact_store=self._artifact_store(),
                runner=self,
                event_emitter=emit_event,
            )
            timeout = _to_seconds(node.attrs.get("timeout"))
            if timeout is None or timeout <= 0:
                return self._invoke_handler_with_contract(handler_type, handler, runtime)

            message = f"handler timed out after {timeout:g}s"
            try:
                with _wall_timeout(timeout):
                    return self._invoke_handler_with_contract(handler_type, handler, runtime)
            except TimeoutError:
                return Outcome(status=OutcomeStatus.FAIL, failure_reason=message)
        finally:
            if entered:
                self._exit_call()

    @contextmanager
    def allow_concurrency(self):
        with self._concurrency_lock:
            self._concurrency_overrides += 1
        try:
            yield
        finally:
            with self._concurrency_lock:
                self._concurrency_overrides = max(0, self._concurrency_overrides - 1)

    def _enter_call(self) -> None:
        with self._concurrency_lock:
            self._active_calls += 1
            if self._active_calls > 1 and self._concurrency_overrides == 0:
                self._active_calls -= 1
                raise RuntimeError(
                    "Concurrent handler execution is only supported inside parallel handlers"
                )

    def _exit_call(self) -> None:
        with self._concurrency_lock:
            self._active_calls = max(0, self._active_calls - 1)

    def set_logs_root(self, logs_root: str | Path | None) -> None:
        if logs_root is None:
            self.logs_root = None
            self.artifact_store = None
            return
        self.logs_root = Path(logs_root)
        self._sync_artifact_store()

    def _artifact_store(self) -> ArtifactStore | None:
        return self.artifact_store

    def _sync_artifact_store(self) -> None:
        if self.logs_root is None:
            self.artifact_store = None
            return
        self.artifact_store = ArtifactStore(base_dir=self.logs_root.parent / "artifacts")

    def _invoke_handler_with_contract(
        self, handler_type: str, handler: Any, runtime: HandlerRuntime
    ) -> Outcome | None:
        lock = self._custom_handler_lock(handler_type, handler)
        if lock is None:
            return _invoke_handler(handler, runtime)

        with lock:
            return _invoke_handler(handler, runtime)

    def _custom_handler_lock(self, handler_type: str, handler: Any) -> threading.Lock | None:
        if handler_type in BUILTIN_HANDLER_TYPES:
            return None
        if _is_declared_thread_safe(handler):
            return None

        key = id(handler)
        with self._custom_handler_locks_guard:
            lock = self._custom_handler_locks.get(key)
            if lock is None:
                lock = threading.Lock()
                self._custom_handler_locks[key] = lock
            return lock


class _SignalTimeout:
    def __init__(self, seconds: float):
        self.seconds = seconds
        self._previous_handler = None
        self._previous_timer = (0.0, 0.0)
        self._enabled = False

    def __enter__(self) -> None:
        if not hasattr(signal, "setitimer"):
            return None
        try:
            self._previous_handler = signal.getsignal(signal.SIGALRM)
            self._previous_timer = signal.getitimer(signal.ITIMER_REAL)
            signal.signal(signal.SIGALRM, _raise_timeout)
            signal.setitimer(signal.ITIMER_REAL, self.seconds)
            self._enabled = True
        except (AttributeError, ValueError):
            self._enabled = False
        return None

    def __exit__(self, exc_type, exc, tb) -> bool:
        if not self._enabled:
            return False
        signal.setitimer(signal.ITIMER_REAL, 0)
        if self._previous_handler is not None:
            signal.signal(signal.SIGALRM, self._previous_handler)
        delay, interval = self._previous_timer
        if delay > 0 or interval > 0:
            signal.setitimer(signal.ITIMER_REAL, delay, interval)
        return False


def _wall_timeout(seconds: float) -> _SignalTimeout:
    return _SignalTimeout(seconds)


def _raise_timeout(signum: int, frame: Any) -> None:
    del signum, frame
    raise TimeoutError


def _to_seconds(attr: Any) -> float | None:
    if not attr:
        return None
    value = attr.value
    if isinstance(value, Duration):
        unit = value.unit
        if unit == "ms":
            return value.value / 1000
        if unit == "s":
            return value.value
        if unit == "m":
            return value.value * 60
        if unit == "h":
            return value.value * 3600
        if unit == "d":
            return value.value * 86400
    if isinstance(value, (int, float)):
        return float(value)
    try:
        return float(str(value))
    except (TypeError, ValueError):
        return None


def _invoke_handler(handler: Any, runtime: HandlerRuntime) -> Outcome | None:
    execute = getattr(handler, "execute", None)
    if not callable(execute):
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="handler does not implement execute(runtime)")
    outcome = execute(runtime)

    if outcome is None:
        return None
    if isinstance(outcome, Outcome):
        return outcome
    return Outcome(status=OutcomeStatus.FAIL, failure_reason="handler returned non-Outcome result")


def _is_declared_thread_safe(handler: Any) -> bool:
    return bool(getattr(handler, "thread_safe", False) or getattr(handler, "stateless", False))
