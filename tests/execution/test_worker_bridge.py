from __future__ import annotations

import io
import json
import threading
from typing import Any

import pytest

from attractor.execution.errors import WorkerAPIError
from attractor.execution.worker_bridge import run_node_process


class BridgeCallbacks:
    def __init__(self) -> None:
        self.events: list[tuple[str, dict[str, Any]]] = []

    def emit_process_event(self, event_type: str, payload: dict[str, Any]) -> None:
        self.events.append((event_type, payload))

    def resolve_process_request(self, request_type: str, payload: dict[str, Any]) -> dict[str, Any] | None:
        if request_type == "human_gate_request":
            return {"value": "YES"}
        if request_type == "child_run_request":
            return {"run_id": payload["child_run_id"], "status": "completed"}
        if request_type == "child_status_request":
            return {"run_id": payload["run_id"], "status": "running"}
        return None


def test_worker_bridge_writes_request_reads_messages_and_replies_to_process_requests() -> None:
    stdin = io.StringIO()
    stdout = io.StringIO(
        "\n".join(
            [
                json.dumps({"type": "event", "event_type": "WorkerProgress", "payload": {"step": "one"}}),
                json.dumps({"type": "human_gate_request", "gate_id": "gate-1"}),
                json.dumps({"type": "child_run_request", "child_run_id": "child-1"}),
                json.dumps({"type": "child_status_request", "run_id": "child-1"}),
                json.dumps(
                    {
                        "type": "result",
                        "outcome": {"status": "success", "context_updates": {}},
                        "context": {"context.done": True},
                        "runtime_metadata": {"pid": 123},
                    }
                ),
            ]
        )
        + "\n"
    )
    callbacks = BridgeCallbacks()

    result = run_node_process(
        request={"node_id": "n1"},
        stdin=stdin,
        stdout=stdout,
        stderr=io.StringIO(""),
        wait=lambda: 0,
        callbacks=callbacks,
    )

    writes = [json.loads(line) for line in stdin.getvalue().strip().splitlines()]
    assert writes == [
        {"node_id": "n1"},
        {"answer": {"value": "YES"}, "type": "human_gate_answer"},
        {"result": {"run_id": "child-1", "status": "completed"}, "type": "child_run_result"},
        {"result": {"run_id": "child-1", "status": "running"}, "type": "child_status_result"},
    ]
    assert callbacks.events == [
        ("node_event", {"process_message_type": "event", "process_event_type": "WorkerProgress", "step": "one"}),
        ("human_gate_request", {"gate_id": "gate-1"}),
        ("child_run_request", {"child_run_id": "child-1"}),
        ("child_status_request", {"run_id": "child-1"}),
    ]
    assert result.outcome == {"status": "success", "context_updates": {}}
    assert result.context == {"context.done": True}
    assert result.runtime_metadata == {"pid": 123}


def test_worker_bridge_waits_for_callback_delivery_before_replying_to_process_request() -> None:
    stdin = io.StringIO()
    stdout = io.StringIO(
        "\n".join(
            [
                json.dumps({"type": "human_gate_request", "gate_id": "gate-1"}),
                json.dumps(
                    {
                        "type": "result",
                        "outcome": {"status": "success", "context_updates": {}},
                    }
                ),
            ]
        )
        + "\n"
    )
    request_emitted = threading.Event()
    callback_delivered = threading.Event()

    class BlockingCallbacks(BridgeCallbacks):
        def emit_process_event(self, event_type: str, payload: dict[str, Any]) -> None:
            super().emit_process_event(event_type, payload)
            request_emitted.set()

        def resolve_process_request(self, request_type: str, payload: dict[str, Any]) -> dict[str, Any] | None:
            assert callback_delivered.wait(timeout=2)
            return {"value": "LATER"}

    callbacks = BlockingCallbacks()
    errors: list[BaseException] = []

    def run_bridge() -> None:
        try:
            run_node_process(
                request={"node_id": "n1"},
                stdin=stdin,
                stdout=stdout,
                stderr=io.StringIO(""),
                wait=lambda: 0,
                callbacks=callbacks,
            )
        except BaseException as exc:  # pragma: no cover - surfaced below.
            errors.append(exc)

    thread = threading.Thread(target=run_bridge)
    thread.start()
    assert request_emitted.wait(timeout=2)
    assert [json.loads(line) for line in stdin.getvalue().strip().splitlines()] == [{"node_id": "n1"}]

    callback_delivered.set()
    thread.join(timeout=2)

    assert errors == []
    writes = [json.loads(line) for line in stdin.getvalue().strip().splitlines()]
    assert writes == [{"node_id": "n1"}, {"answer": {"value": "LATER"}, "type": "human_gate_answer"}]


@pytest.mark.parametrize(
    ("stdout", "return_code", "expected_code"),
    [
        ("not-json\n", 0, "invalid_process_message"),
        ("", 0, "missing_process_result"),
        ("", 7, "node_process_crashed"),
        (json.dumps({"type": "result", "context": {}}) + "\n", 0, "invalid_process_result"),
        (
            json.dumps({"type": "result", "outcome": {"status": "bogus", "context_updates": {}}}) + "\n",
            0,
            "invalid_process_result",
        ),
    ],
)
def test_worker_bridge_converts_process_failures_to_structured_errors(
    stdout: str,
    return_code: int,
    expected_code: str,
) -> None:
    with pytest.raises(WorkerAPIError) as exc_info:
        run_node_process(
            request={"node_id": "n1"},
            stdin=io.StringIO(),
            stdout=io.StringIO(stdout),
            stderr=io.StringIO("boom"),
            wait=lambda: return_code,
            callbacks=BridgeCallbacks(),
        )

    assert exc_info.value.code == expected_code
