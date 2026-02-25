from __future__ import annotations

import asyncio
from dataclasses import dataclass
import threading
import uuid
import os
import shutil
from datetime import datetime
from pathlib import Path
import subprocess
from typing import Callable, Dict, Optional

from fastapi import FastAPI, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse
from pydantic import BaseModel

from attractor.dsl import DotParseError, Diagnostic, DiagnosticSeverity, parse_dot, validate_graph
from attractor.engine import Context, PipelineExecutor
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import CodergenBackend
from attractor.interviewer import AutoApproveInterviewer
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
        self._pause_requested = False
        self._abort_requested = False

    def reset(self) -> None:
        with self._lock:
            self._pause_requested = False
            self._abort_requested = False

    def request_pause(self) -> None:
        with self._lock:
            self._pause_requested = True

    def request_abort(self) -> None:
        with self._lock:
            self._abort_requested = True

    def poll(self) -> Optional[str]:
        with self._lock:
            if self._abort_requested:
                return "abort"
            if self._pause_requested:
                return "pause"
        return None


class HumanGateBroker:
    def __init__(self):
        self._lock = threading.Lock()
        self._pending: Dict[str, Dict[str, object]] = {}

    def request(
        self,
        question: Question,
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

    def answer(self, gate_id: str, selected_value: str) -> bool:
        with self._lock:
            entry = self._pending.get(gate_id)
            if not entry:
                return False
            entry["answer"] = selected_value
            entry["event"].set()
            return True


HUMAN_BROKER = HumanGateBroker()
RUN_CONTROL = ExecutionControl()


class WebInterviewer(Interviewer):
    def __init__(self, broker: HumanGateBroker, emit: Callable[[dict], None], flow_name: str):
        self._broker = broker
        self._emit = emit
        self._flow_name = flow_name

    def ask(self, question: Question) -> Answer:
        node_id = str(question.metadata.get("node_id", "")).strip()
        if not node_id and question.title.lower().startswith("human gate:"):
            node_id = question.title.split(":", 1)[1].strip()
        return self._broker.request(question, node_id, self._flow_name, self._emit)


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


def _append_runtime_log(message: str) -> None:
    if not RUNTIME.last_working_directory or not RUNTIME.last_run_id:
        return
    run_log_path = (
        Path(RUNTIME.last_working_directory)
        / ".attractor"
        / "runs"
        / RUNTIME.last_run_id
        / "run.log"
    )
    try:
        run_log_path.parent.mkdir(parents=True, exist_ok=True)
        timestamp = datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S")
        with run_log_path.open("a", encoding="utf-8") as f:
            f.write(f"[{timestamp} UTC] {message}\n")
    except Exception:
        pass


class RunRequest(BaseModel):
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
    question_id: str
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

    def run(self, node_id: str, prompt: str, context: Context) -> bool:
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
        proc = subprocess.run(
            cmd,
            cwd=self.working_dir,
            capture_output=True,
            text=True,
        )
        if proc.stdout.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stdout.strip()}"})
        if proc.stderr.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stderr.strip()}"})
        return proc.returncode == 0


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


@app.post("/run")
async def run_pipeline(req: RunRequest):
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = str(exc)
        await manager.broadcast({"type": "log", "msg": f"❌ Parse error: {exc}"})
        parse_diag = {
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
        }
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
        for diag in errors:
            await manager.broadcast({"type": "log", "msg": f"❌ {diag.rule_id}: {diag.message}"})
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

    await manager.broadcast(
        {
            "type": "graph",
            **_graph_payload(graph),
        }
    )

    loop = asyncio.get_running_loop()

    async def broadcast_log(message: str) -> None:
        _append_runtime_log(message)
        await manager.broadcast({"type": "log", "msg": message})

    def emit(message: dict):
        if message.get("type") == "log":
            _append_runtime_log(str(message.get("msg", "")))
        asyncio.run_coroutine_threadsafe(manager.broadcast(message), loop)

    if req.backend != "codex":
        return {
            "status": "validation_error",
            "error": "Unsupported backend. This build requires backend='codex'.",
        }

    backend: CodergenBackend = LocalCodexCliBackend(
        working_dir,
        emit,
        model=selected_model or None,
    )

    interviewer: Interviewer
    if manager.active_connections:
        interviewer = WebInterviewer(HUMAN_BROKER, emit, flow_name)
    else:
        interviewer = AutoApproveInterviewer()

    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )
    runner = BroadcastingRunner(HandlerRunner(graph, registry), emit)

    run_root = Path(working_dir) / ".attractor" / "runs" / run_id
    checkpoint_file = str(run_root / "state.json")
    logs_root = str(run_root / "logs")

    goal_attr = graph.graph_attrs.get("goal")
    context = Context(values={"graph.goal": str(goal_attr.value) if goal_attr else ""})

    RUN_CONTROL.reset()
    RUNTIME.status = "running"
    RUNTIME.last_error = ""
    RUNTIME.last_working_directory = working_dir
    RUNTIME.last_model = display_model
    RUNTIME.last_flow_name = flow_name
    RUNTIME.last_run_id = run_id

    await manager.broadcast({"type": "runtime", "status": RUNTIME.status})

    await manager.broadcast(
        {
            "type": "run_meta",
            "working_directory": working_dir,
            "model": display_model,
            "flow_name": flow_name,
            "run_id": run_id,
        }
    )
    await broadcast_log(
        f"[System] Launching run {run_id} in {working_dir} with model: {display_model}"
    )

    async def _run():
        nonlocal context
        try:
            executor = PipelineExecutor(
                graph,
                runner,
                logs_root=logs_root,
                checkpoint_file=checkpoint_file,
                control=RUN_CONTROL.poll,
            )
            result = await asyncio.to_thread(
                executor.run,
                context,
                resume=True,
            )
            RUNTIME.status = result.status
            RUNTIME.last_completed_nodes = result.completed_nodes
            emit({"type": "runtime", "status": RUNTIME.status})
            await broadcast_log(f"Pipeline {result.status}")
        except Exception as exc:  # noqa: BLE001
            import traceback
            traceback.print_exc()
            RUNTIME.status = "failed"
            RUNTIME.last_error = str(exc)
            emit({"type": "runtime", "status": RUNTIME.status})
            await broadcast_log(f"⚠️ Pipeline Aborted: {exc}")
        finally:
            RUN_CONTROL.reset()

    asyncio.create_task(_run())
    return {
        "status": "started",
        "working_directory": working_dir,
        "model": display_model,
        "run_id": run_id,
    }


@app.post("/pause")
async def pause_pipeline():
    if RUNTIME.status not in {"running", "pause_requested"}:
        return {"status": "ignored", "runtime": RUNTIME.status}
    RUN_CONTROL.request_pause()
    RUNTIME.status = "pause_requested"
    await manager.broadcast({"type": "runtime", "status": RUNTIME.status})
    _append_runtime_log("[System] Pause requested. Will pause after current node.")
    await manager.broadcast({"type": "log", "msg": "[System] Pause requested. Will pause after current node."})
    return {"status": "pause_requested"}


@app.post("/abort")
async def abort_pipeline():
    if RUNTIME.status not in {"running", "pause_requested"}:
        return {"status": "ignored", "runtime": RUNTIME.status}
    RUN_CONTROL.request_abort()
    RUNTIME.status = "abort_requested"
    RUNTIME.last_error = "aborted_by_user"
    await manager.broadcast({"type": "runtime", "status": RUNTIME.status})
    _append_runtime_log("[System] Abort requested. Stopping after current node.")
    await manager.broadcast({"type": "log", "msg": "[System] Abort requested. Stopping after current node."})
    return {"status": "abort_requested"}


@app.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    run_root = Path(req.working_directory).resolve() / ".attractor" / "runs"
    if run_root.exists():
        shutil.rmtree(run_root, ignore_errors=True)
    return {"status": "reset"}


@app.post("/human/answer")
async def submit_human_answer(req: HumanAnswerRequest):
    ok = HUMAN_BROKER.answer(req.question_id, req.selected_value)
    if not ok:
        raise HTTPException(status_code=404, detail="Unknown human gate request")
    return {"status": "accepted"}


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
