from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
import json
import subprocess
from typing import Any, Protocol, TextIO

from attractor.engine.outcome import FailureKind, OutcomeStatus

from .errors import WorkerAPIError


@dataclass(frozen=True)
class WorkerProcessFailure:
    code: str
    message: str
    retryable: bool = False
    details: dict[str, Any] | None = None


class WorkerNodeProcessCallbacks(Protocol):
    def emit_process_event(self, event_type: str, payload: dict[str, Any]) -> None:
        ...

    def resolve_process_request(self, request_type: str, payload: dict[str, Any]) -> dict[str, Any] | None:
        ...


@dataclass(frozen=True)
class WorkerNodeProcessResult:
    outcome: dict[str, Any]
    context: dict[str, Any]
    runtime_metadata: dict[str, Any]


def run_node_process(
    *,
    request: dict[str, Any],
    stdin: TextIO,
    stdout: TextIO,
    wait: Callable[[], int],
    callbacks: WorkerNodeProcessCallbacks,
    stderr: TextIO | None = None,
) -> WorkerNodeProcessResult:
    try:
        stdin.write(json.dumps(request, sort_keys=True) + "\n")
        stdin.flush()
    except OSError as exc:
        raise WorkerAPIError(
            "node_process_stdin_failed",
            "Worker bridge could not write the node request to process stdin.",
            500,
            retryable=True,
            details={"error": str(exc)},
        ) from exc

    final_payload: dict[str, Any] | None = None
    try:
        for line in stdout:
            payload = _decode_process_message(line)
            kind = str(payload.get("type") or "")
            if kind == "result":
                final_payload = payload
                continue
            if kind in {"event", "progress", "log"}:
                callbacks.emit_process_event(_worker_event_type(kind), _event_payload(kind, payload))
                continue
            if kind in {"human_gate_request", "child_run_request", "child_status_request"}:
                callbacks.emit_process_event(kind, {key: value for key, value in payload.items() if key != "type"})
                response = callbacks.resolve_process_request(kind, payload)
                if response is not None:
                    stdin.write(json.dumps(_process_response(kind, response), sort_keys=True) + "\n")
                    stdin.flush()
                continue
            raise WorkerAPIError(
                "invalid_process_message",
                "Worker node process emitted an unsupported message type.",
                502,
                details={"type": kind},
            )
    except json.JSONDecodeError as exc:
        raise WorkerAPIError(
            "invalid_process_message",
            "Worker node process emitted invalid JSON.",
            502,
            details={"error": str(exc)},
        ) from exc
    except OSError as exc:
        raise WorkerAPIError(
            "node_process_io_failed",
            "Worker bridge failed while exchanging process protocol messages.",
            502,
            retryable=True,
            details={"error": str(exc)},
        ) from exc

    stderr_text = stderr.read() if stderr is not None else ""
    return_code = wait()
    if return_code != 0:
        raise WorkerAPIError(
            "node_process_crashed",
            "Worker node process exited before producing a valid result.",
            502,
            retryable=True,
            details={"return_code": return_code, "stderr": stderr_text.strip()},
        )
    if final_payload is None:
        raise WorkerAPIError(
            "missing_process_result",
            "Worker node process exited without a final result.",
            502,
            retryable=True,
            details={"stderr": stderr_text.strip()},
        )
    outcome = final_payload.get("outcome")
    if not isinstance(outcome, dict) or not _is_serialized_outcome(outcome):
        raise WorkerAPIError(
            "invalid_process_result",
            "Worker node process result did not include a valid serialized Outcome.",
            502,
            details={"result": final_payload},
        )
    context = final_payload.get("context")
    metadata = final_payload.get("runtime_metadata")
    return WorkerNodeProcessResult(
        outcome=dict(outcome),
        context=dict(context) if isinstance(context, dict) else {},
        runtime_metadata=dict(metadata) if isinstance(metadata, dict) else {},
    )


class SubprocessWorkerNodeProcess:
    def __init__(self, command: list[str], *, cwd: str | None = None) -> None:
        self._proc = subprocess.Popen(
            command,
            cwd=cwd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        assert self._proc.stdin is not None
        assert self._proc.stdout is not None
        self.stdin = self._proc.stdin
        self.stdout = self._proc.stdout
        self.stderr = self._proc.stderr

    def wait(self) -> int:
        return int(self._proc.wait())

    def cancel(self) -> None:
        if self._proc.poll() is not None:
            return
        self._proc.terminate()
        try:
            self._proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self._proc.kill()


def _decode_process_message(line: str) -> dict[str, Any]:
    payload = json.loads(line)
    if not isinstance(payload, dict):
        raise WorkerAPIError(
            "invalid_process_message",
            "Worker node process emitted a non-object JSON message.",
            502,
            details={"message": payload},
        )
    return payload


def _worker_event_type(kind: str) -> str:
    if kind == "log":
        return "worker_log"
    return "node_event"


def _event_payload(kind: str, payload: dict[str, Any]) -> dict[str, Any]:
    event_payload = payload.get("payload")
    if not isinstance(event_payload, dict):
        event_payload = {key: value for key, value in payload.items() if key not in {"type", "event_type"}}
    return {
        "process_message_type": kind,
        "process_event_type": str(payload.get("event_type") or kind),
        **dict(event_payload),
    }


def _process_response(kind: str, response: dict[str, Any]) -> dict[str, Any]:
    if kind == "human_gate_request":
        return {"type": "human_gate_answer", "answer": response}
    if kind == "child_run_request":
        return {"type": "child_run_result", "result": response}
    return {"type": "child_status_result", "result": response}


def _is_serialized_outcome(outcome: dict[str, Any]) -> bool:
    try:
        OutcomeStatus(str(outcome.get("status") or ""))
    except ValueError:
        return False
    if "preferred_label" in outcome and not isinstance(outcome["preferred_label"], str):
        return False
    if "suggested_next_ids" in outcome and (
        not isinstance(outcome["suggested_next_ids"], list)
        or not all(isinstance(item, str) for item in outcome["suggested_next_ids"])
    ):
        return False
    if "context_updates" in outcome and not isinstance(outcome["context_updates"], dict):
        return False
    if "notes" in outcome and not isinstance(outcome["notes"], str):
        return False
    if "failure_reason" in outcome and not isinstance(outcome["failure_reason"], str):
        return False
    if "failure_kind" in outcome:
        try:
            FailureKind(str(outcome["failure_kind"]))
        except ValueError:
            return False
    return True
