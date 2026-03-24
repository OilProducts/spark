from __future__ import annotations

import asyncio
import threading
import uuid
from dataclasses import dataclass, field
from typing import Callable, Dict, List, Optional

from fastapi import WebSocket

from attractor.engine import Context
from attractor.interviewer.base import Interviewer
from attractor.interviewer.models import Answer, AnswerValue, Question


class ConnectionManager:
    def __init__(self):
        self.active_connections: list[WebSocket] = []

    async def connect(self, websocket: WebSocket):
        await websocket.accept()
        self.active_connections.append(websocket)

    def disconnect(self, websocket: WebSocket):
        if websocket in self.active_connections:
            self.active_connections.remove(websocket)

    async def broadcast(self, message: dict):
        for connection in list(self.active_connections):
            try:
                await connection.send_json(message)
            except Exception:
                pass


class ExecutionControl:
    def __init__(self):
        self._lock = threading.Lock()
        self._cancel_requested = False

    def reset(self) -> None:
        with self._lock:
            self._cancel_requested = False

    def request_cancel(self) -> None:
        with self._lock:
            self._cancel_requested = True

    def poll(self) -> Optional[str]:
        with self._lock:
            if self._cancel_requested:
                return "abort"
        return None


class HumanGateBroker:
    def __init__(self):
        self._lock = threading.Lock()
        self._pending: Dict[str, Dict[str, object]] = {}

    def request(
        self,
        question: Question,
        run_id: str,
        node_id: str,
        flow_name: str,
        emit: Callable[[dict], None],
    ) -> Answer:
        gate_id = uuid.uuid4().hex
        event = threading.Event()
        with self._lock:
            self._pending[gate_id] = {
                "event": event,
                "answer": None,
                "run_id": run_id,
                "node_id": node_id,
                "flow_name": flow_name,
                "prompt": question.text,
                "options": [
                    {"label": opt.label, "value": opt.value} for opt in question.options
                ],
            }

        emit(
            {
                "type": "state",
                "node": node_id,
                "status": "waiting",
            }
        )
        emit(
            {
                "type": "human_gate",
                "question_id": gate_id,
                "node_id": node_id,
                "flow_name": flow_name,
                "prompt": question.text,
                "options": [
                    {"label": opt.label, "value": opt.value} for opt in question.options
                ],
            }
        )

        timeout_seconds = question.timeout_seconds
        wait_timeout = timeout_seconds if timeout_seconds and timeout_seconds > 0 else None
        responded = event.wait(wait_timeout)
        with self._lock:
            entry = self._pending.pop(gate_id, {})
            selected = entry.get("answer") if entry else None

        if not responded and question.default is not None:
            return question.default
        if selected:
            return Answer(selected_values=[str(selected)])
        return Answer(value=AnswerValue.TIMEOUT)

    def answer(self, run_id: str, gate_id: str, selected_value: str) -> bool:
        with self._lock:
            entry = self._pending.get(gate_id)
            if not entry:
                return False
            if str(entry.get("run_id", "")) != run_id:
                return False
            entry["answer"] = selected_value
            entry["event"].set()
            return True

    def list_for_run(self, run_id: str) -> List[Dict[str, object]]:
        with self._lock:
            payload: List[Dict[str, object]] = []
            for gate_id, entry in self._pending.items():
                if str(entry.get("run_id", "")) != run_id:
                    continue
                if entry.get("answer") is not None:
                    continue
                payload.append(
                    {
                        "question_id": gate_id,
                        "run_id": str(entry.get("run_id", "")),
                        "node_id": str(entry.get("node_id", "")),
                        "flow_name": str(entry.get("flow_name", "")),
                        "prompt": str(entry.get("prompt", "")),
                        "options": list(entry.get("options") or []),
                    }
                )
            return payload


class WebInterviewer(Interviewer):
    def __init__(
        self,
        broker: HumanGateBroker,
        emit: Callable[[dict], None],
        flow_name: str,
        run_id: str,
    ):
        self._broker = broker
        self._emit = emit
        self._flow_name = flow_name
        self._run_id = run_id

    def ask(self, question: Question) -> Answer:
        node_id = str(question.metadata.get("node_id", "")).strip()
        if not node_id:
            node_id = question.stage.strip()
        return self._broker.request(question, self._run_id, node_id, self._flow_name, self._emit)


class PipelineEventHub:
    def __init__(self):
        self._lock = threading.Lock()
        self._history: Dict[str, List[dict]] = {}
        self._subscribers: Dict[str, List[asyncio.Queue[dict]]] = {}
        self._max_history = 500

    async def publish(self, run_id: str, event: dict) -> None:
        queues: List[asyncio.Queue[dict]] = []
        with self._lock:
            history = self._history.setdefault(run_id, [])
            history.append(event)
            if len(history) > self._max_history:
                del history[:-self._max_history]
            queues = list(self._subscribers.get(run_id, []))
        for queue in queues:
            try:
                queue.put_nowait(event)
            except asyncio.QueueFull:
                try:
                    queue.get_nowait()
                except asyncio.QueueEmpty:
                    continue
                try:
                    queue.put_nowait(event)
                except asyncio.QueueFull:
                    continue

    def history(self, run_id: str) -> List[dict]:
        with self._lock:
            return list(self._history.get(run_id, []))

    def subscribe(self, run_id: str) -> asyncio.Queue[dict]:
        queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=256)
        with self._lock:
            self._subscribers.setdefault(run_id, []).append(queue)
        return queue

    def subscribe_with_history(self, run_id: str) -> tuple[asyncio.Queue[dict], List[dict]]:
        queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=256)
        with self._lock:
            history = list(self._history.get(run_id, []))
            self._subscribers.setdefault(run_id, []).append(queue)
        return queue, history

    def unsubscribe(self, run_id: str, queue: asyncio.Queue[dict]) -> None:
        with self._lock:
            listeners = self._subscribers.get(run_id)
            if not listeners:
                return
            if queue in listeners:
                listeners.remove(queue)
            if not listeners:
                self._subscribers.pop(run_id, None)


@dataclass
class ActiveRun:
    run_id: str
    flow_name: str
    working_directory: str
    model: str
    status: str = "running"
    outcome: str | None = None
    outcome_reason_code: str | None = None
    outcome_reason_message: str | None = None
    last_error: str = ""
    completed_nodes: List[str] = field(default_factory=list)
    control: ExecutionControl = field(default_factory=ExecutionControl)


@dataclass
class RuntimeState:
    status: str = "idle"
    outcome: str | None = None
    outcome_reason_code: str | None = None
    outcome_reason_message: str | None = None
    last_error: str = ""
    last_working_directory: str = ""
    last_model: str = ""
    last_completed_nodes: list[str] = None
    last_flow_name: str = ""
    last_run_id: str = ""


class BroadcastingRunner:
    def __init__(self, delegate, emit):
        self.delegate = delegate
        self.emit = emit

    def set_logs_root(self, logs_root):
        setter = getattr(self.delegate, "set_logs_root", None)
        if callable(setter):
            setter(logs_root)

    def __call__(self, node_id: str, prompt: str, context: Context):
        self.emit({"type": "state", "node": node_id, "status": "running"})
        self.emit({"type": "log", "msg": f"[{node_id}] running"})
        outcome = self.delegate(node_id, prompt, context)
        status_map = {
            "success": "success",
            "partial_success": "success",
            "retry": "running",
            "fail": "failed",
        }
        mapped = status_map.get(outcome.status.value, "failed")
        self.emit({"type": "state", "node": node_id, "status": mapped})
        if outcome.failure_reason:
            self.emit({"type": "log", "msg": f"[{node_id}] {outcome.failure_reason}"})
        return outcome
