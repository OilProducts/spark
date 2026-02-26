from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
import threading
import uuid
import os
import shutil
import re
import selectors
import time
from datetime import datetime
from pathlib import Path
import subprocess
from typing import Callable, Dict, List, Optional

from fastapi import FastAPI, HTTPException, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, StreamingResponse
from pydantic import BaseModel

from attractor.dsl import DotParseError, Diagnostic, DiagnosticSeverity, parse_dot, validate_graph
from attractor.engine import Context, PipelineExecutor
from attractor.graphviz_export import export_graphviz_artifact
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import CodergenBackend
from attractor.interviewer.base import Interviewer
from attractor.interviewer.models import Answer, Question
from attractor.transforms import (
    GoalVariableTransform,
    ModelStylesheetTransform,
    TransformPipeline,
)


app = FastAPI()
PROJECT_ROOT = Path(__file__).resolve().parents[2]
FRONTEND_DIST = PROJECT_ROOT / "frontend" / "dist"
FRONTEND_DIST_INDEX = FRONTEND_DIST / "index.html"
LEGACY_INDEX = PROJECT_ROOT / "index.html"


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


manager = ConnectionManager()


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
                "prompt": question.prompt,
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
                "prompt": question.prompt,
                "options": [
                    {"label": opt.label, "value": opt.value} for opt in question.options
                ],
            }
        )

        event.wait()
        with self._lock:
            entry = self._pending.pop(gate_id, {})
            selected = entry.get("answer") if entry else None

        if selected:
            return Answer(selected_values=[str(selected)])
        return Answer()

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


HUMAN_BROKER = HumanGateBroker()


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
        if not node_id and question.title.lower().startswith("human gate:"):
            node_id = question.title.split(":", 1)[1].strip()
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
                continue

    def history(self, run_id: str) -> List[dict]:
        with self._lock:
            return list(self._history.get(run_id, []))

    def subscribe(self, run_id: str) -> asyncio.Queue[dict]:
        queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=256)
        with self._lock:
            self._subscribers.setdefault(run_id, []).append(queue)
        return queue

    def unsubscribe(self, run_id: str, queue: asyncio.Queue[dict]) -> None:
        with self._lock:
            listeners = self._subscribers.get(run_id)
            if not listeners:
                return
            if queue in listeners:
                listeners.remove(queue)
            if not listeners:
                self._subscribers.pop(run_id, None)


EVENT_HUB = PipelineEventHub()


@dataclass
class ActiveRun:
    run_id: str
    flow_name: str
    working_directory: str
    model: str
    status: str = "running"
    last_error: str = ""
    completed_nodes: List[str] = field(default_factory=list)
    control: ExecutionControl = field(default_factory=ExecutionControl)


@dataclass
class RuntimeState:
    status: str = "idle"
    last_error: str = ""
    last_working_directory: str = ""
    last_model: str = ""
    last_completed_nodes: list[str] = None
    last_flow_name: str = ""
    last_run_id: str = ""


RUNTIME = RuntimeState(last_completed_nodes=[])
ACTIVE_RUNS_LOCK = threading.Lock()
ACTIVE_RUNS: Dict[str, ActiveRun] = {}


@dataclass
class RunRecord:
    run_id: str
    flow_name: str
    status: str
    result: Optional[str]
    working_directory: str
    model: str
    started_at: str
    ended_at: Optional[str] = None
    last_error: str = ""
    token_usage: Optional[int] = None

    def to_dict(self) -> Dict[str, object]:
        return {
            "run_id": self.run_id,
            "flow_name": self.flow_name,
            "status": self.status,
            "result": self.result,
            "working_directory": self.working_directory,
            "model": self.model,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "last_error": self.last_error,
            "token_usage": self.token_usage,
        }

    @classmethod
    def from_dict(cls, data: Dict[str, object]) -> "RunRecord":
        return cls(
            run_id=str(data.get("run_id", "")),
            flow_name=str(data.get("flow_name", "")),
            status=str(data.get("status", "unknown")),
            result=data.get("result") if data.get("result") is not None else None,
            working_directory=str(data.get("working_directory", "")),
            model=str(data.get("model", "")),
            started_at=str(data.get("started_at", "")),
            ended_at=data.get("ended_at") if data.get("ended_at") is not None else None,
            last_error=str(data.get("last_error", "")),
            token_usage=int(data["token_usage"]) if data.get("token_usage") is not None else None,
        )


RUN_HISTORY_LOCK = threading.Lock()
RUNS_ROOT = PROJECT_ROOT / ".attractor" / "runs"


def _run_root(run_id: str) -> Path:
    return RUNS_ROOT / run_id


def _run_meta_path(run_id: str) -> Path:
    return _run_root(run_id) / "run.json"


def _write_run_meta(record: RunRecord) -> None:
    run_meta_path = _run_meta_path(record.run_id)
    try:
        run_meta_path.parent.mkdir(parents=True, exist_ok=True)
        with run_meta_path.open("w", encoding="utf-8") as f:
            json.dump(record.to_dict(), f, indent=2, sort_keys=True)
    except Exception:
        pass


def _read_run_meta(path: Path) -> Optional[RunRecord]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        return RunRecord.from_dict(payload)
    except Exception:
        return None


def _normalize_run_status(status: str) -> str:
    if status == "fail":
        return "failed"
    if status in {"aborted", "abort_requested"}:
        return {"aborted": "canceled", "abort_requested": "cancel_requested"}[status]
    if status == "cancelled":
        return "canceled"
    return status


def _record_run_start(run_id: str, flow_name: str, working_directory: str, model: str) -> None:
    record = RunRecord(
        run_id=run_id,
        flow_name=flow_name,
        status="running",
        result=None,
        working_directory=working_directory,
        model=model,
        started_at=datetime.utcnow().isoformat(timespec="seconds") + "Z",
    )
    with RUN_HISTORY_LOCK:
        _write_run_meta(record)


TOKEN_LINE_RE = re.compile(r"tokens used\\s*[:=]?\\s*(\\d[\\d,]*)", re.IGNORECASE)
TOKEN_NUMBER_ONLY_RE = re.compile(r"^\\d[\\d,]*$")


def _extract_token_usage(run_id: str) -> Optional[int]:
    run_log_path = _run_root(run_id) / "run.log"
    if not run_log_path.exists():
        return None
    try:
        lines = run_log_path.read_text(encoding="utf-8", errors="ignore").splitlines()
    except Exception:
        return None
    total = 0
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        match = TOKEN_LINE_RE.search(line)
        if match:
            total += int(match.group(1).replace(",", ""))
        elif line.lower() == "tokens used" and index + 1 < len(lines):
            next_line = lines[index + 1].strip()
            if TOKEN_NUMBER_ONLY_RE.match(next_line):
                total += int(next_line.replace(",", ""))
                index += 1
        index += 1
    return total if total > 0 else None


def _record_run_end(run_id: str, working_directory: str, status: str, last_error: str = "") -> None:
    normalized_status = _normalize_run_status(status)
    with RUN_HISTORY_LOCK:
        record = _read_run_meta(_run_meta_path(run_id))
        if not record:
            record = RunRecord(
                run_id=run_id,
                flow_name="",
                status=normalized_status,
                result=normalized_status,
                working_directory=working_directory,
                model="",
                started_at="",
            )
        record.status = normalized_status
        record.result = normalized_status
        record.ended_at = datetime.utcnow().isoformat(timespec="seconds") + "Z"
        record.last_error = last_error
        record.token_usage = _extract_token_usage(run_id)
        _write_run_meta(record)


def _record_run_status(run_id: str, status: str, last_error: str = "") -> None:
    normalized_status = _normalize_run_status(status)
    with RUN_HISTORY_LOCK:
        record = _read_run_meta(_run_meta_path(run_id))
        if not record:
            return
        record.status = normalized_status
        record.result = normalized_status
        if last_error:
            record.last_error = last_error
        _write_run_meta(record)


def _append_run_log(run_id: str, message: str) -> None:
    run_log_path = _run_root(run_id) / "run.log"
    try:
        run_log_path.parent.mkdir(parents=True, exist_ok=True)
        timestamp = datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S")
        with run_log_path.open("a", encoding="utf-8") as f:
            f.write(f"[{timestamp} UTC] {message}\n")
    except Exception:
        pass


async def _publish_run_event(run_id: str, message: dict) -> None:
    payload = dict(message)
    payload.setdefault("run_id", run_id)
    if payload.get("type") == "log":
        _append_run_log(run_id, str(payload.get("msg", "")))
    await manager.broadcast(payload)
    await EVENT_HUB.publish(run_id, payload)


def _set_active_run_status(run_id: str, status: str, *, last_error: Optional[str] = None) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.status = status
        if last_error is not None:
            run.last_error = last_error


def _set_active_run_completed_nodes(run_id: str, completed_nodes: List[str]) -> None:
    with ACTIVE_RUNS_LOCK:
        run = ACTIVE_RUNS.get(run_id)
        if not run:
            return
        run.completed_nodes = list(completed_nodes)


def _get_active_run(run_id: str) -> Optional[ActiveRun]:
    with ACTIVE_RUNS_LOCK:
        return ACTIVE_RUNS.get(run_id)


def _pop_active_run(run_id: str) -> Optional[ActiveRun]:
    with ACTIVE_RUNS_LOCK:
        return ACTIVE_RUNS.pop(run_id, None)


class PipelineStartRequest(BaseModel):
    flow_content: str
    working_directory: str = "./workspace"
    backend: str = "codex"
    model: Optional[str] = None
    flow_name: Optional[str] = None


class PreviewRequest(BaseModel):
    flow_content: str


class SaveFlowRequest(BaseModel):
    name: str
    content: str


class ResetRequest(BaseModel):
    working_directory: str = "./workspace"


class HumanAnswerRequest(BaseModel):
    selected_value: str


DEFAULT_FLOW = """digraph SoftwareFactory {
    start [shape=Mdiamond, label="Start"];
    setup [shape=box, prompt="Initialize project"];
    build [shape=box, prompt="Build app"];
    done [shape=Msquare, label="Done"];

    start -> setup -> build -> done;
}"""


class LocalCodexCliBackend(CodergenBackend):
    def __init__(self, working_dir: str, emit, model: Optional[str] = None):
        self.working_dir = working_dir
        self.emit = emit
        self.model = model

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> bool:
        cmd = [
            "codex",
            "exec",
            "-C",
            self.working_dir,
            "-s",
            "danger-full-access",
        ]
        if self.model:
            cmd.extend(["-m", self.model])
        cmd.append(prompt)
        try:
            proc = subprocess.run(
                cmd,
                cwd=self.working_dir,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            self.emit({"type": "log", "msg": f"[{node_id}] timeout after {timeout}s"})
            return False
        if proc.stdout.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stdout.strip()}"})
        if proc.stderr.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stderr.strip()}"})
        return proc.returncode == 0


class LocalCodexAppServerBackend(CodergenBackend):
    def __init__(self, working_dir: str, emit, model: Optional[str] = None):
        self.working_dir = working_dir
        self.emit = emit
        self.model = model

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> bool:
        cmd = ["codex", "app-server"]
        deadline = time.time() + timeout if timeout else None
        agent_chunks: list[str] = []
        command_chunks: list[str] = []
        last_token_total: Optional[int] = None
        turn_status: Optional[str] = None
        turn_error: Optional[str] = None

        def log_line(message: str) -> None:
            if message:
                self.emit({"type": "log", "msg": f"[{node_id}] {message}"})

        try:
            proc = subprocess.Popen(
                cmd,
                cwd=self.working_dir,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
        except FileNotFoundError:
            log_line("codex app-server not found on PATH")
            return False

        selector = selectors.DefaultSelector()
        if proc.stdout is not None:
            selector.register(proc.stdout, selectors.EVENT_READ)

        def send_json(payload: dict) -> None:
            if proc.stdin is None:
                return
            proc.stdin.write(json.dumps(payload) + "\n")
            proc.stdin.flush()

        request_id = 0

        def send_request(method: str, params: Optional[dict]) -> int:
            nonlocal request_id
            request_id += 1
            payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
            if params is not None:
                payload["params"] = params
            send_json(payload)
            return request_id

        def send_response(req_id: object, result: Optional[dict] = None, error: Optional[dict] = None) -> None:
            payload = {"jsonrpc": "2.0", "id": req_id}
            if error is not None:
                payload["error"] = error
            else:
                payload["result"] = result or {}
            send_json(payload)

        def read_line(wait: float) -> Optional[str]:
            if proc.stdout is None:
                return None
            if wait < 0:
                wait = 0
            events = selector.select(timeout=wait)
            if not events:
                return None
            line = proc.stdout.readline()
            if not line:
                return None
            return line.rstrip("\n")

        def handle_server_request(message: dict) -> None:
            method = message.get("method")
            req_id = message.get("id")
            if method == "item/commandExecution/requestApproval":
                send_response(req_id, {"decision": "acceptForSession"})
                return
            if method == "item/fileChange/requestApproval":
                send_response(req_id, {"decision": "acceptForSession"})
                return
            send_response(req_id, error={"code": -32000, "message": f"Unsupported request: {method}"})

        def handle_notification(message: dict) -> None:
            nonlocal last_token_total, turn_status, turn_error
            method = message.get("method")
            params = message.get("params") or {}
            if method == "item/agentMessage/delta":
                delta = params.get("delta") or ""
                if delta:
                    agent_chunks.append(delta)
                return
            if method == "codex/event/agent_message_delta":
                delta = (params.get("msg") or {}).get("delta") or ""
                if delta:
                    agent_chunks.append(delta)
                return
            if method == "codex/event/agent_message":
                msg = (params.get("msg") or {}).get("message")
                if msg:
                    agent_chunks.clear()
                    agent_chunks.append(msg)
                return
            if method == "item/commandExecution/outputDelta":
                delta = params.get("delta") or ""
                if delta:
                    command_chunks.append(delta)
                return
            if method == "thread/tokenUsage/updated":
                token_usage = params.get("tokenUsage") or {}
                total_tokens = (token_usage.get("total") or {}).get("totalTokens")
                if isinstance(total_tokens, int):
                    last_token_total = total_tokens
                return
            if method == "codex/event/token_count":
                info = (params.get("msg") or {}).get("info") or {}
                total_tokens = (info.get("total_token_usage") or {}).get("total_tokens")
                if isinstance(total_tokens, int):
                    last_token_total = total_tokens
                return
            if method == "error":
                turn_status = "failed"
                turn_error = (params.get("message") or "App server error")
                return
            if method == "turn/completed":
                turn = params.get("turn") or {}
                turn_status = turn.get("status")
                if turn_status == "failed":
                    err = turn.get("error") or {}
                    turn_error = err.get("message") or turn_error

        def wait_for_response(target_id: int) -> Optional[dict]:
            while True:
                if deadline is not None and time.time() > deadline:
                    return None
                line = read_line(0.1)
                if line is None:
                    if proc.poll() is not None:
                        return None
                    continue
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    log_line(line)
                    continue
                if "id" in message and "method" in message:
                    handle_server_request(message)
                    continue
                if message.get("id") == target_id:
                    return message
                if "method" in message:
                    handle_notification(message)

        def wait_for_turn_completion() -> bool:
            while True:
                if deadline is not None and time.time() > deadline:
                    return False
                line = read_line(0.1)
                if line is None:
                    if proc.poll() is not None:
                        return False
                    continue
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    log_line(line)
                    continue
                if "id" in message and "method" in message:
                    handle_server_request(message)
                    continue
                if "method" in message:
                    handle_notification(message)
                    if message.get("method") == "turn/completed":
                        return True

        try:
            init_id = send_request(
                "initialize",
                {"clientInfo": {"name": "sparkspawn", "version": "0.1"}},
            )
            init_response = wait_for_response(init_id)
            if not init_response or init_response.get("error"):
                log_line("app-server initialize failed")
                return False

            thread_params = {
                "cwd": self.working_dir,
                "sandbox": "danger-full-access",
                "ephemeral": True,
            }
            if self.model:
                thread_params["model"] = self.model
            thread_id = send_request("thread/start", thread_params)
            thread_response = wait_for_response(thread_id)
            if not thread_response or thread_response.get("error"):
                log_line("app-server thread/start failed")
                return False
            thread = (thread_response.get("result") or {}).get("thread") or {}
            thread_uuid = thread.get("id")
            if not thread_uuid:
                log_line("app-server thread/start did not return thread id")
                return False

            turn_params = {
                "threadId": thread_uuid,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": self.working_dir,
            }
            if self.model:
                turn_params["model"] = self.model
            turn_request_id = send_request("turn/start", turn_params)
            turn_response = wait_for_response(turn_request_id)
            if not turn_response or turn_response.get("error"):
                log_line("app-server turn/start failed")
                return False

            completed = wait_for_turn_completion()
            if not completed:
                log_line("app-server turn timed out or exited early")
                return False

            if turn_status and turn_status != "completed":
                if turn_error:
                    log_line(turn_error)
                return False

            agent_text = "".join(agent_chunks).strip()
            if agent_text:
                log_line(agent_text)
            command_text = "".join(command_chunks).strip()
            if command_text:
                log_line(command_text)
            if last_token_total is not None:
                log_line(f"tokens used: {last_token_total}")
            return True
        finally:
            try:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        proc.kill()
            except Exception:
                pass


class BroadcastingRunner:
    def __init__(self, delegate, emit):
        self.delegate = delegate
        self.emit = emit

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


@app.get("/")
async def get_ui():
    if FRONTEND_DIST_INDEX.exists():
        return FileResponse(FRONTEND_DIST_INDEX)
    return FileResponse(LEGACY_INDEX)


@app.get("/assets/{asset_path:path}")
async def get_frontend_asset(asset_path: str):
    file_path = FRONTEND_DIST / "assets" / asset_path
    if not file_path.exists():
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


@app.get("/vite.svg")
async def get_frontend_vite_icon():
    file_path = FRONTEND_DIST / "vite.svg"
    if not file_path.exists():
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    await manager.connect(websocket)
    try:
        while True:
            await websocket.receive_text()
    except WebSocketDisconnect:
        manager.disconnect(websocket)


@app.get("/status")
async def get_status():
    return {
        "status": RUNTIME.status,
        "last_error": RUNTIME.last_error,
        "last_working_directory": RUNTIME.last_working_directory,
        "last_model": RUNTIME.last_model,
        "last_completed_nodes": RUNTIME.last_completed_nodes,
        "last_flow_name": RUNTIME.last_flow_name,
        "last_run_id": RUNTIME.last_run_id,
    }


@app.get("/runs")
async def list_runs():
    if not RUNS_ROOT.exists():
        return {"runs": []}

    records: List[RunRecord] = []
    for run_dir in RUNS_ROOT.iterdir():
        if not run_dir.is_dir():
            continue
        meta_path = run_dir / "run.json"
        record = _read_run_meta(meta_path)
        if record:
            records.append(record)
            continue

        run_id = run_dir.name
        record = RunRecord(
            run_id=run_id,
            flow_name="",
            status="unknown",
            result=None,
            working_directory="",
            model="",
            started_at="",
        )
        run_log_path = run_dir / "run.log"
        if run_log_path.exists():
            record.token_usage = _extract_token_usage(run_id)
            try:
                lines = run_log_path.read_text(encoding="utf-8", errors="ignore").splitlines()
            except Exception:
                lines = []
            if lines:
                first_line = lines[0]
                timestamp_match = re.search(r"^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) UTC\]", first_line)
                if timestamp_match:
                    record.started_at = f"{timestamp_match.group(1).replace(' ', 'T')}Z"

                status = None
                for line in reversed(lines):
                    status_match = re.search(r"Pipeline\s+(\w+)", line)
                    if status_match:
                        status = _normalize_run_status(status_match.group(1))
                        break
                    if "Pipeline Aborted" in line:
                        status = "canceled"
                        break

                if status:
                    record.status = status
                    record.result = status
                    last_line = lines[-1]
                    last_timestamp = re.search(r"^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) UTC\]", last_line)
                    if last_timestamp:
                        record.ended_at = f"{last_timestamp.group(1).replace(' ', 'T')}Z"
                else:
                    record.status = "running"
        records.append(record)

    def _sort_key(item: RunRecord) -> str:
        return item.started_at or item.ended_at or ""

    records.sort(key=_sort_key, reverse=True)
    return {"runs": [record.to_dict() for record in records]}


def _graph_payload(graph) -> dict:
    def _attr_value(attrs: Dict[str, object], key: str, default: Optional[object] = None):
        attr = attrs.get(key)
        if attr is None:
            return default
        value = getattr(attr, "value", default)
        if hasattr(value, "raw"):
            return value.raw
        return value

    return {
        "nodes": [
            {
                "id": n.node_id,
                "label": _attr_value(n.attrs, "label", n.node_id),
                "shape": _attr_value(n.attrs, "shape"),
                "prompt": _attr_value(n.attrs, "prompt"),
                "tool_command": _attr_value(n.attrs, "tool_command"),
                "join_policy": _attr_value(n.attrs, "join_policy"),
                "error_policy": _attr_value(n.attrs, "error_policy"),
                "max_parallel": _attr_value(n.attrs, "max_parallel"),
                "type": _attr_value(n.attrs, "type"),
                "max_retries": _attr_value(n.attrs, "max_retries"),
                "goal_gate": _attr_value(n.attrs, "goal_gate"),
                "retry_target": _attr_value(n.attrs, "retry_target"),
                "fallback_retry_target": _attr_value(n.attrs, "fallback_retry_target"),
                "fidelity": _attr_value(n.attrs, "fidelity"),
                "thread_id": _attr_value(n.attrs, "thread_id"),
                "class": _attr_value(n.attrs, "class"),
                "timeout": _attr_value(n.attrs, "timeout"),
                "llm_model": _attr_value(n.attrs, "llm_model"),
                "llm_provider": _attr_value(n.attrs, "llm_provider"),
                "reasoning_effort": _attr_value(n.attrs, "reasoning_effort"),
                "auto_status": _attr_value(n.attrs, "auto_status"),
                "allow_partial": _attr_value(n.attrs, "allow_partial"),
            }
            for n in graph.nodes.values()
        ],
        "graph_attrs": {
            "goal": _attr_value(graph.graph_attrs, "goal"),
            "label": _attr_value(graph.graph_attrs, "label"),
            "model_stylesheet": _attr_value(graph.graph_attrs, "model_stylesheet"),
            "default_max_retry": _attr_value(graph.graph_attrs, "default_max_retry"),
            "retry_target": _attr_value(graph.graph_attrs, "retry_target"),
            "fallback_retry_target": _attr_value(graph.graph_attrs, "fallback_retry_target"),
            "default_fidelity": _attr_value(graph.graph_attrs, "default_fidelity"),
            "ui_default_llm_model": _attr_value(graph.graph_attrs, "ui_default_llm_model"),
            "ui_default_llm_provider": _attr_value(graph.graph_attrs, "ui_default_llm_provider"),
            "ui_default_reasoning_effort": _attr_value(graph.graph_attrs, "ui_default_reasoning_effort"),
        },
        "edges": [
            {
                "from": e.source,
                "to": e.target,
                "label": _attr_value(e.attrs, "label"),
                "condition": _attr_value(e.attrs, "condition"),
                "weight": _attr_value(e.attrs, "weight"),
                "fidelity": _attr_value(e.attrs, "fidelity"),
                "thread_id": _attr_value(e.attrs, "thread_id"),
                "loop_restart": _attr_value(e.attrs, "loop_restart"),
            }
            for e in graph.edges
        ],
    }


def _diagnostic_payload(diagnostic: Diagnostic) -> dict:
    payload = {
        "rule_id": diagnostic.rule_id,
        "severity": diagnostic.severity.value,
        "message": diagnostic.message,
        "line": diagnostic.line,
    }
    if diagnostic.node_id is not None:
        payload["node_id"] = diagnostic.node_id
    if diagnostic.edge is not None:
        payload["edge"] = list(diagnostic.edge)
    if diagnostic.fix is not None:
        payload["fix"] = diagnostic.fix
    return payload


@app.post("/preview")
async def preview_pipeline(req: PreviewRequest):
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        parse_diag = {
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
        }
        return {
            "status": "parse_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    graph = pipeline.apply(graph)

    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]

    payload = {
        "status": "ok" if not errors else "validation_error",
        "graph": _graph_payload(graph),
        "diagnostics": [_diagnostic_payload(d) for d in diagnostics],
        "errors": [_diagnostic_payload(d) for d in errors],
    }
    return payload


async def _start_pipeline(req: PipelineStartRequest) -> dict:
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = str(exc)
        parse_diag = {
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
        }
        if RUNTIME.last_run_id:
            await _publish_run_event(RUNTIME.last_run_id, {"type": "log", "msg": f"❌ Parse error: {exc}"})
        return {
            "status": "validation_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    if errors:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = errors[0].message
        return {
            "status": "validation_error",
            "diagnostics": [_diagnostic_payload(d) for d in diagnostics],
            "errors": [_diagnostic_payload(d) for d in errors],
        }

    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    graph = pipeline.apply(graph)

    os.makedirs(req.working_directory, exist_ok=True)
    working_dir = str(Path(req.working_directory).resolve())
    selected_model = (req.model or "").strip()
    flow_name = (req.flow_name or "").strip()
    display_model = selected_model or "codex default (config/profile)"
    run_id = uuid.uuid4().hex

    await _publish_run_event(
        run_id,
        {
            "type": "graph",
            **_graph_payload(graph),
        },
    )

    loop = asyncio.get_running_loop()

    def emit(message: dict):
        asyncio.run_coroutine_threadsafe(_publish_run_event(run_id, message), loop)

    if req.backend != "codex":
        return {
            "status": "validation_error",
            "error": "Unsupported backend. This build requires backend='codex'.",
        }

    backend: CodergenBackend = LocalCodexAppServerBackend(
        working_dir,
        emit,
        model=selected_model or None,
    )

    interviewer: Interviewer = WebInterviewer(HUMAN_BROKER, emit, flow_name, run_id)

    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )
    runner = BroadcastingRunner(HandlerRunner(graph, registry), emit)

    run_root = _run_root(run_id)
    checkpoint_file = str(run_root / "state.json")
    logs_root = str(run_root / "logs")
    graphviz_export = export_graphviz_artifact(req.flow_content, run_root)

    goal_attr = graph.graph_attrs.get("goal")
    context = Context(values={"graph.goal": str(goal_attr.value) if goal_attr else ""})

    with ACTIVE_RUNS_LOCK:
        ACTIVE_RUNS[run_id] = ActiveRun(
            run_id=run_id,
            flow_name=flow_name,
            working_directory=working_dir,
            model=display_model,
            status="running",
        )

    RUNTIME.status = "running"
    RUNTIME.last_error = ""
    RUNTIME.last_working_directory = working_dir
    RUNTIME.last_model = display_model
    RUNTIME.last_flow_name = flow_name
    RUNTIME.last_run_id = run_id

    _record_run_start(run_id, flow_name, working_dir, display_model)

    await _publish_run_event(run_id, {"type": "runtime", "status": RUNTIME.status})

    await _publish_run_event(
        run_id,
        {
            "type": "run_meta",
            "working_directory": working_dir,
            "model": display_model,
            "flow_name": flow_name,
            "run_id": run_id,
            "graph_dot_path": str(graphviz_export.dot_path),
            "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
        },
    )
    if graphviz_export.error:
        await _publish_run_event(
            run_id,
            {
                "type": "log",
                "msg": f"[System] Graph render unavailable: {graphviz_export.error}",
            },
        )
    await _publish_run_event(
        run_id,
        {
            "type": "log",
            "msg": f"[System] Launching run {run_id} in {working_dir} with model: {display_model}",
        },
    )

    async def _run():
        try:
            active = _get_active_run(run_id)
            control = active.control if active else ExecutionControl()
            executor = PipelineExecutor(
                graph,
                runner,
                logs_root=logs_root,
                checkpoint_file=checkpoint_file,
                control=control.poll,
            )
            result = await asyncio.to_thread(
                executor.run,
                context,
                resume=True,
            )
            final_status = _normalize_run_status(result.status)
            _set_active_run_status(run_id, final_status)
            _set_active_run_completed_nodes(run_id, result.completed_nodes)
            RUNTIME.status = final_status
            RUNTIME.last_completed_nodes = result.completed_nodes
            await _publish_run_event(run_id, {"type": "runtime", "status": final_status})
            _record_run_end(run_id, working_dir, final_status)
            await _publish_run_event(run_id, {"type": "log", "msg": f"Pipeline {final_status}"})
        except Exception as exc:  # noqa: BLE001
            import traceback
            traceback.print_exc()
            _set_active_run_status(run_id, "failed", last_error=str(exc))
            RUNTIME.status = "failed"
            RUNTIME.last_error = str(exc)
            await _publish_run_event(run_id, {"type": "runtime", "status": "failed"})
            _record_run_end(run_id, working_dir, "failed", str(exc))
            await _publish_run_event(run_id, {"type": "log", "msg": f"⚠️ Pipeline Failed: {exc}"})
        finally:
            _pop_active_run(run_id)

    asyncio.create_task(_run())
    return {
        "status": "started",
        "pipeline_id": run_id,
        "run_id": run_id,
        "working_directory": working_dir,
        "model": display_model,
        "graph_dot_path": str(graphviz_export.dot_path),
        "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
    }


@app.post("/pipelines")
async def create_pipeline(req: PipelineStartRequest):
    return await _start_pipeline(req)


@app.get("/pipelines/{pipeline_id}")
async def get_pipeline(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if active:
        return {
            "pipeline_id": pipeline_id,
            "status": active.status,
            "flow_name": active.flow_name,
            "working_directory": active.working_directory,
            "model": active.model,
            "last_error": active.last_error,
            "completed_nodes": active.completed_nodes,
        }

    record = _read_run_meta(_run_meta_path(pipeline_id))
    if not record:
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    return {
        "pipeline_id": pipeline_id,
        "status": record.status,
        "flow_name": record.flow_name,
        "working_directory": record.working_directory,
        "model": record.model,
        "last_error": record.last_error,
        "completed_nodes": [],
        "started_at": record.started_at,
        "ended_at": record.ended_at,
        "result": record.result,
    }


@app.get("/pipelines/{pipeline_id}/events")
async def pipeline_events(pipeline_id: str, request: Request):
    active = _get_active_run(pipeline_id)
    existing = _read_run_meta(_run_meta_path(pipeline_id))
    if not active and not existing and not EVENT_HUB.history(pipeline_id):
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    queue = EVENT_HUB.subscribe(pipeline_id)
    history = EVENT_HUB.history(pipeline_id)

    async def stream():
        try:
            for event in history:
                yield f"data: {json.dumps(event)}\n\n"

            while True:
                if await request.is_disconnected():
                    break
                try:
                    event = await asyncio.wait_for(queue.get(), timeout=15.0)
                    yield f"data: {json.dumps(event)}\n\n"
                except asyncio.TimeoutError:
                    yield ": keepalive\n\n"
        finally:
            EVENT_HUB.unsubscribe(pipeline_id, queue)

    return StreamingResponse(stream(), media_type="text/event-stream")


@app.post("/pipelines/{pipeline_id}/cancel")
async def cancel_pipeline(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active:
        return {"status": "ignored", "pipeline_id": pipeline_id}
    active.control.request_cancel()
    _set_active_run_status(pipeline_id, "cancel_requested", last_error="cancel_requested_by_user")
    _record_run_status(pipeline_id, "cancel_requested", "cancel_requested_by_user")
    RUNTIME.status = "cancel_requested"
    RUNTIME.last_error = "cancel_requested_by_user"
    await _publish_run_event(pipeline_id, {"type": "runtime", "status": "cancel_requested"})
    await _publish_run_event(
        pipeline_id,
        {"type": "log", "msg": "[System] Cancel requested. Stopping after current node."},
    )
    return {"status": "cancel_requested", "pipeline_id": pipeline_id}


@app.get("/pipelines/{pipeline_id}/questions")
async def list_pipeline_questions(pipeline_id: str):
    return {"questions": HUMAN_BROKER.list_for_run(pipeline_id)}


@app.post("/pipelines/{pipeline_id}/questions/{question_id}/answer")
async def submit_pipeline_answer(pipeline_id: str, question_id: str, req: HumanAnswerRequest):
    ok = HUMAN_BROKER.answer(pipeline_id, question_id, req.selected_value)
    if not ok:
        raise HTTPException(status_code=404, detail="Unknown question for pipeline")
    return {"status": "accepted", "pipeline_id": pipeline_id, "question_id": question_id}


@app.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    if RUNS_ROOT.exists():
        shutil.rmtree(RUNS_ROOT, ignore_errors=True)
    return {"status": "reset"}


@app.get("/api/flows")
async def list_flows():
    flows_dir = Path("flows")
    flows_dir.mkdir(exist_ok=True)
    return [f.name for f in flows_dir.glob("*.dot")]


@app.get("/api/flows/{name}")
async def get_flow(name: str):
    flow_path = Path("flows") / name
    if not flow_path.exists():
        return {"error": "Flow not found"}, 404
    return {"name": name, "content": flow_path.read_text()}


@app.post("/api/flows")
async def save_flow(req: SaveFlowRequest):
    flows_dir = Path("flows")
    flows_dir.mkdir(exist_ok=True)
    flow_path = flows_dir / req.name
    if not flow_path.name.endswith(".dot"):
        flow_path = flow_path.with_suffix(".dot")
    flow_path.write_text(req.content)
    return {"status": "saved", "name": flow_path.name}


@app.delete("/api/flows/{flow_name}")
async def delete_flow(flow_name: str):
    filepath = Path("flows") / flow_name
    if filepath.exists():
        filepath.unlink()
        return {"status": "deleted"}
    return {"error": "Flow not found"}, 404
