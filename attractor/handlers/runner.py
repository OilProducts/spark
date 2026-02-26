from __future__ import annotations

from contextlib import contextmanager
import signal
from dataclasses import dataclass, field
from pathlib import Path
import threading
from typing import Any

from attractor.dsl.models import Duration
from attractor.dsl.models import DotGraph
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus

from .base import HandlerRuntime
from .registry import HandlerRegistry


@dataclass
class HandlerRunner:
    graph: DotGraph
    registry: HandlerRegistry
    logs_root: Path | None = None
    _concurrency_lock: threading.Lock = field(default_factory=threading.Lock, init=False, repr=False)
    _active_calls: int = field(default=0, init=False, repr=False)
    _concurrency_overrides: int = field(default=0, init=False, repr=False)

    def __call__(self, node_id: str, prompt: str, context: Context) -> Outcome:
        entered = False
        try:
            self._enter_call()
            entered = True
            node = self.graph.nodes[node_id]
            outgoing = [edge for edge in self.graph.edges if edge.source == node_id]
            handler = self.registry.resolve(node)
            runtime = HandlerRuntime(
                node_id=node_id,
                node=node,
                prompt=prompt,
                node_attrs=node.attrs,
                outgoing_edges=outgoing,
                context=context,
                graph=self.graph,
                logs_root=self.logs_root,
                runner=self,
            )
            timeout = _to_seconds(node.attrs.get("timeout"))
            if timeout is None or timeout <= 0:
                return _invoke_handler(handler, runtime)

            message = f"handler timed out after {timeout:g}s"
            try:
                with _wall_timeout(timeout):
                    return _invoke_handler(handler, runtime)
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
            return
        self.logs_root = Path(logs_root)


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


def _invoke_handler(handler: Any, runtime: HandlerRuntime) -> Outcome:
    execute = getattr(handler, "execute", None)
    if callable(execute):
        outcome = execute(runtime)
    else:
        # Backward compatibility for plugin handlers still using the old contract.
        run = getattr(handler, "run", None)
        if not callable(run):
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="handler does not implement execute(runtime)")
        outcome = run(runtime)

    if isinstance(outcome, Outcome):
        return outcome
    return Outcome(status=OutcomeStatus.FAIL, failure_reason="handler returned non-Outcome result")
