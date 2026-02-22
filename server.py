import asyncio
import json
import re
import os
from abc import ABC, abstractmethod
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse
from pydantic import BaseModel

app = FastAPI()


# ==========================================
# WEBSOCKET MANAGER (Live Updates to UI)
# ==========================================
class ConnectionManager:
    def __init__(self):
        self.active_connections: list[WebSocket] = []

    async def connect(self, websocket: WebSocket):
        await websocket.accept()
        self.active_connections.append(websocket)

    def disconnect(self, websocket: WebSocket):
        self.active_connections.remove(websocket)

    async def broadcast(self, message: dict):
        for connection in self.active_connections:
            try:
                await connection.send_json(message)
            except:
                pass


manager = ConnectionManager()


# ==========================================
# THE CODERGEN BACKEND & MOCK
# ==========================================
class CodergenBackend(ABC):
    @abstractmethod
    async def run(self, node_id: str, prompt: str) -> bool:
        pass


class MockCodexBackend(CodergenBackend):
    async def run(self, node_id: str, prompt: str) -> bool:
        await manager.broadcast(
            {"type": "log", "msg": f"[{node_id}] AI Worker started: '{prompt}'"})
        await manager.broadcast({"type": "state", "node": node_id, "status": "running"})

        # Simulate parallel AI processing time
        await asyncio.sleep(3)

        # Simulate a random failure for demonstration (10% chance)
        # import random
        # if random.random() < 0.1:
        #     await manager.broadcast({"type": "log", "msg": f"[{node_id}] ❌ AI Worker encountered an error!"})
        #     return False

        await manager.broadcast(
            {"type": "log", "msg": f"[{node_id}] ✔ AI Worker finished successfully."})
        await manager.broadcast({"type": "state", "node": node_id, "status": "success"})
        return True


class CodexAppServerBackend(CodergenBackend):
    def __init__(self, codex_bin: str = "codex", working_dir: str = "."):
        self.codex_bin = codex_bin
        self.working_dir = os.path.abspath(working_dir)
        self.process = None
        self._msg_id = 0
        self.thread_id = None
        self._init_lock = asyncio.Lock()

        self.pending_requests = {}
        self.turn_events = {}

    async def _start(self):
        self.process = await asyncio.create_subprocess_exec(
            self.codex_bin, "app-server",
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            cwd=self.working_dir
        )

        asyncio.create_task(self._reader_task())
        loop = asyncio.get_running_loop()

        # -----------------------------------------------------------------
        # 1. INITIALIZE (Per Codex App Server Specs)
        # Payload requires params.clientInfo with both name AND version.
        # -----------------------------------------------------------------
        init_id = self._next_id()
        self.pending_requests[init_id] = loop.create_future()
        self._send({
            "id": init_id,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "Attractor Engine",
                    "version": "1.0.0"
                }
            }
        })

        init_res = await self.pending_requests[init_id]
        if "error" in init_res:
            raise RuntimeError(f"Codex Initialize Error: {init_res['error']}")

        # Notification: must immediately follow initialize
        self._send({"method": "initialized", "params": {}})

        # -----------------------------------------------------------------
        # 2. START THREAD
        # payload: { "model": "gpt-5.1-codex" }
        # -----------------------------------------------------------------
        thread_req_id = self._next_id()
        self.pending_requests[thread_req_id] = loop.create_future()
        self._send({
            "id": thread_req_id,
            "method": "thread/start",
            "params": {"model": "gpt-5.3-codex-spark"}
        })

        thread_res = await self.pending_requests[thread_req_id]
        if "error" in thread_res:
            raise RuntimeError(f"Codex Thread Start Error: {thread_res['error']}")

        self.thread_id = thread_res.get("result", {}).get("thread", {}).get("id")
        await manager.broadcast({"type": "log",
                                 "msg": f"[System] Connected to Codex App Server (Thread: {self.thread_id})"})

    def _next_id(self):
        self._msg_id += 1
        return self._msg_id

    def _send(self, payload: dict):
        # Codex App Server omits the "jsonrpc": "2.0" header on the wire
        self.process.stdin.write((json.dumps(payload) + "\n").encode('utf-8'))

    async def _reader_task(self):
        """Background loop reading standard JSONL outputs."""
        while True:
            line = await self.process.stdout.readline()
            if not line:
                break

            try:
                msg = json.loads(line.decode('utf-8').strip())
                msg_id = msg.get("id")
                method = msg.get("method")
                params = msg.get("params", {})

                # A. Handle Responses with IDs (turn/start, thread/start)
                if msg_id in self.pending_requests:
                    self.pending_requests[msg_id].set_result(msg)
                    continue

                    # B. Handle Turn Completed Notifications
                if method == "turn/completed":
                    turn_id = params.get("turn", {}).get("id")
                    if turn_id in self.turn_events:
                        self.turn_events[turn_id]["status"] = params.get("turn", {}).get("status")
                        self.turn_events[turn_id]["event"].set()

                # C. Live UI Logs: Tool Executions
                elif method == "item/started":
                    item_type = params.get("item", {}).get("type")
                    if item_type == "toolExecution":
                        tool_name = params.get("item", {}).get("toolExecution", {}).get("name")
                        await manager.broadcast(
                            {"type": "log", "msg": f"🤖 Codex executing tool: {tool_name}"})

                # C. Live UI Logs: Text Generation
                elif method == "item/agentMessage/delta":
                    text_delta = params.get("delta", "")
                    if text_delta:
                        # Optional: Print raw text generation to your actual terminal
                        print(text_delta, end="", flush=True)

                # D. Handle Human-in-the-loop server requests
                elif method == "window/showMessageRequest":
                    self._send({"id": msg_id, "result": {"action": "Approve"}})

            except json.JSONDecodeError:
                continue

    async def run(self, node_id: str, prompt: str) -> bool:
        async with self._init_lock:
            if not self.process:
                await self._start()

        await manager.broadcast({"type": "state", "node": node_id, "status": "running"})
        await manager.broadcast(
            {"type": "log", "msg": f"[{node_id}] Sending task to Codex App Server..."})

        req_id = self._next_id()
        loop = asyncio.get_running_loop()
        self.pending_requests[req_id] = loop.create_future()

        # -----------------------------------------------------------------
        # 3. START TURN
        # Payload requires threadId and an input array of discriminated unions
        # -----------------------------------------------------------------
        self._send({
            "id": req_id,
            "method": "turn/start",
            "params": {
                "threadId": self.thread_id,
                "input": [{"type": "text", "text": prompt}]  # <-- Fixed format
            }
        })

        turn_start_res = await self.pending_requests[req_id]

        if "error" in turn_start_res:
            error_msg = turn_start_res['error'].get('message', str(turn_start_res['error']))
            await manager.broadcast(
                {"type": "log", "msg": f"[{node_id}] ❌ Codex Error: {error_msg}"})
            return False

        turn_id = turn_start_res.get("result", {}).get("turn", {}).get("id")

        if not turn_id:
            await manager.broadcast(
                {"type": "log", "msg": f"[{node_id}] ❌ Failed to parse Turn ID."})
            return False

        # Wait for the turn/completed notification matching this ID
        self.turn_events[turn_id] = {"status": None, "event": asyncio.Event()}
        await self.turn_events[turn_id]["event"].wait()

        final_status = self.turn_events[turn_id]["status"]

        # "success" or "completed" depending on the minor version of the CLI you installed
        success = (final_status in ["success", "completed"])

        status_color = "success" if success else "failed"
        await manager.broadcast({"type": "state", "node": node_id, "status": status_color})

        del self.turn_events[turn_id]
        return success

# ==========================================
# ATTRACTOR ENGINE & PARSER
# ==========================================
def parse_dot_blueprint(dot_string: str):
    nodes = {}
    dependencies = {}

    for line in dot_string.splitlines():
        node_match = re.search(r'(\w+)\s*\[.*prompt="([^"]+)"', line)
        if node_match:
            node_id, prompt = node_match.groups()
            nodes[node_id] = {"prompt": prompt}
            if node_id not in dependencies:
                dependencies[node_id] = []

        edge_match = re.search(r'(\w+)\s*->\s*(\w+)', line)
        if edge_match:
            src, dst = edge_match.groups()
            if dst not in dependencies:
                dependencies[dst] = []
            dependencies[dst].append(src)

    return nodes, dependencies


class AttractorEngine:
    def __init__(self, backend: CodergenBackend, working_dir: str):
        self.backend = backend
        self.working_dir = os.path.abspath(working_dir)
        os.makedirs(self.working_dir, exist_ok=True)
        self.checkpoint_file = os.path.join(self.working_dir, "attractor.state.json")

    def _load_checkpoint(self) -> list:
        if os.path.exists(self.checkpoint_file):
            with open(self.checkpoint_file, 'r') as f:
                return json.load(f)
        return []

    def _save_checkpoint(self, completed_nodes: list):
        with open(self.checkpoint_file, 'w') as f:
            json.dump(completed_nodes, f)

    async def execute_pipeline(self, dot_blueprint: str):
        nodes, dependencies = parse_dot_blueprint(dot_blueprint)
        events = {node: asyncio.Event() for node in nodes}
        completed_nodes = self._load_checkpoint()

        # Tell UI about the graph structure
        await manager.broadcast({
            "type": "graph",
            "nodes": [{"id": n, "label": n} for n in nodes.keys()],
            "edges": [{"from": src, "to": dst} for dst, srcs in dependencies.items() for src in
                      srcs]
        })

        async def _run_node(node_id: str, data: dict):
            # Wait for dependencies
            for dep in dependencies.get(node_id, []):
                await events[dep].wait()

            # Checkpoint logic
            if node_id in completed_nodes:
                await manager.broadcast(
                    {"type": "log", "msg": f"[{node_id}] ⏭️ Skipping (Loaded from checkpoint)."})
                await manager.broadcast({"type": "state", "node": node_id, "status": "skipped"})
                events[node_id].set()
                return

            # Execute
            success = await self.backend.run(node_id, data["prompt"])
            if success:
                completed_nodes.append(node_id)
                self._save_checkpoint(completed_nodes)
                events[node_id].set()
            else:
                await manager.broadcast({"type": "state", "node": node_id, "status": "failed"})
                await manager.broadcast(
                    {"type": "log", "msg": f"[{node_id}] ❌ FAILED. Halting pipeline."})
                raise RuntimeError(f"Pipeline failed at {node_id}")

        await manager.broadcast({"type": "log", "msg": "🚀 Starting Pipeline Execution..."})
        try:
            async with asyncio.TaskGroup() as tg:
                for node_id, data in nodes.items():
                    tg.create_task(_run_node(node_id, data))
            await manager.broadcast({"type": "log", "msg": "🎉 Pipeline complete!"})
        except Exception as e:
            await manager.broadcast({"type": "log", "msg": "⚠️ Pipeline Aborted!"})

            # Unwrap the Python 3.11+ ExceptionGroup to surface the real errors
            if isinstance(e, BaseExceptionGroup):
                for i, sub_exc in enumerate(e.exceptions, 1):
                    await manager.broadcast({
                        "type": "log",
                        "msg": f"   ❌ [Error {i}] {type(sub_exc).__name__}: {str(sub_exc)}"
                    })
            else:
                # Fallback for standard exceptions
                await manager.broadcast({
                    "type": "log",
                    "msg": f"   ❌ [Error] {type(e).__name__}: {str(e)}"
                })


# ==========================================
# FASTAPI ENDPOINTS
# ==========================================
DEFAULT_BLUEPRINT = """digraph SoftwareFactory {
    setup [prompt="Initialize Node.js project, install React and Express"];
    backend [prompt="Write the Express.js server logic in server.js"];
    frontend [prompt="Write the React UI in App.js to consume the backend"];
    test [prompt="Write jest tests for frontend and backend and execute them"];

    setup -> backend;
    setup -> frontend;
    backend -> test;
    frontend -> test;
}"""


class RunRequest(BaseModel):
    blueprint: str
    working_directory: str = "./workspace"


class ResetRequest(BaseModel):
    working_directory: str = "./workspace"


@app.get("/")
async def get_ui():
    return FileResponse("index.html")


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    await manager.connect(websocket)
    try:
        while True:
            await websocket.receive_text()
    except WebSocketDisconnect:
        manager.disconnect(websocket)


@app.post("/run")
async def run_pipeline(req: RunRequest):
    os.makedirs(req.working_directory, exist_ok=True)
    backend = CodexAppServerBackend(
        codex_bin="codex",
        working_dir=req.working_directory
    )
    # If using the Mock backend while testing, use: backend = MockCodexBackend()
    engine = AttractorEngine(backend=backend, working_dir=req.working_directory)

    asyncio.create_task(engine.execute_pipeline(req.blueprint))
    return {"status": "started"}


@app.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    target_state = os.path.join(
        os.path.abspath(req.working_directory),
        "attractor.state.json"
    )
    if os.path.exists(target_state):
        os.remove(target_state)
    return {"status": "reset"}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="127.0.0.1", port=8000)
