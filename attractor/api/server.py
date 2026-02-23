from __future__ import annotations

import asyncio
from dataclasses import dataclass
import os
from pathlib import Path
import subprocess
from typing import Dict, Optional

from fastapi import FastAPI, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse
from pydantic import BaseModel

from attractor.dsl import DotParseError, DiagnosticSeverity, parse_dot, validate_graph
from attractor.engine import Context, PipelineExecutor
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import CodergenBackend
from attractor.interviewer import AutoApproveInterviewer
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


@dataclass
class RuntimeState:
    status: str = "idle"
    last_error: str = ""
    last_working_directory: str = ""
    last_model: str = ""
    last_completed_nodes: list[str] = None


RUNTIME = RuntimeState(last_completed_nodes=[])


class RunRequest(BaseModel):
    flow_content: str
    working_directory: str = "./workspace"
    backend: str = "codex"
    model: Optional[str] = None


class PreviewRequest(BaseModel):
    flow_content: str


class SaveFlowRequest(BaseModel):
    name: str
    content: str


class ResetRequest(BaseModel):
    working_directory: str = "./workspace"


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
        cmd = ["codex", "exec"]
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
            }
            for n in graph.nodes.values()
        ],
        "edges": [{"from": e.source, "to": e.target} for e in graph.edges],
    }


@app.post("/preview")
async def preview_pipeline(req: PreviewRequest):
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        return {"status": "parse_error", "error": str(exc)}

    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    graph = pipeline.apply(graph)

    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]

    payload = {
        "status": "ok" if not errors else "validation_error",
        "graph": _graph_payload(graph),
        "errors": [
            {
                "rule_id": d.rule_id,
                "message": d.message,
                "line": d.line,
            }
            for d in errors
        ],
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
        return {"status": "validation_error", "error": str(exc)}

    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    if errors:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = errors[0].message
        for diag in errors:
            await manager.broadcast({"type": "log", "msg": f"❌ {diag.rule_id}: {diag.message}"})
        return {
            "status": "validation_error",
            "errors": [
                {
                    "rule_id": d.rule_id,
                    "message": d.message,
                    "line": d.line,
                }
                for d in errors
            ],
        }

    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    graph = pipeline.apply(graph)

    os.makedirs(req.working_directory, exist_ok=True)
    working_dir = str(Path(req.working_directory).resolve())
    selected_model = (req.model or "").strip()
    display_model = selected_model or "codex default (config/profile)"

    await manager.broadcast(
        {
            "type": "graph",
            **_graph_payload(graph),
        }
    )

    loop = asyncio.get_running_loop()

    def emit(message: dict):
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

    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=AutoApproveInterviewer(),
    )
    runner = BroadcastingRunner(HandlerRunner(graph, registry), emit)

    checkpoint_file = str(Path(working_dir) / "attractor.state.json")
    logs_root = str(Path(working_dir) / ".attractor" / "logs")

    goal_attr = graph.graph_attrs.get("goal")
    context = Context(values={"graph.goal": str(goal_attr.value) if goal_attr else ""})

    RUNTIME.status = "running"
    RUNTIME.last_error = ""
    RUNTIME.last_working_directory = working_dir
    RUNTIME.last_model = display_model

    await manager.broadcast(
        {
            "type": "run_meta",
            "working_directory": working_dir,
            "model": display_model,
        }
    )
    await manager.broadcast(
        {
            "type": "log",
            "msg": f"[System] Launching in {working_dir} with model: {display_model}",
        }
    )

    async def _run():
        nonlocal context
        try:
            executor = PipelineExecutor(
                graph,
                runner,
                logs_root=logs_root,
                checkpoint_file=checkpoint_file,
            )
            result = await asyncio.to_thread(
                executor.run,
                context,
                resume=True,
            )
            RUNTIME.status = result.status
            RUNTIME.last_completed_nodes = result.completed_nodes
            await manager.broadcast({"type": "log", "msg": f"Pipeline {result.status}"})
        except Exception as exc:  # noqa: BLE001
            import traceback
            traceback.print_exc()
            RUNTIME.status = "failed"
            RUNTIME.last_error = str(exc)
            await manager.broadcast({"type": "log", "msg": f"⚠️ Pipeline Aborted: {exc}"})

    asyncio.create_task(_run())
    return {"status": "started", "working_directory": working_dir, "model": display_model}


@app.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    target_state = Path(req.working_directory).resolve() / "attractor.state.json"
    if target_state.exists():
        target_state.unlink()
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
