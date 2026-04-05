from __future__ import annotations

from contextlib import asynccontextmanager
import shutil
import subprocess
import sys
from pathlib import Path

from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse

import attractor.api.pipeline_runs as pipeline_runs
import attractor.api.server as attractor_server
from spark_app.ui import resolve_ui_asset_path, resolve_ui_index_path
from workspace.attractor_client import AttractorApiClient
from workspace.api import WorkspaceApiDependencies, create_workspace_router
from workspace.triggers import TriggerRuntime


DEFAULT_EXECUTION_PLANNING_FLOW = "plan-generation.dot"
DEFAULT_EXECUTION_DISPATCH_FLOW = "spec-implementation/implement-spec.dot"

app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)
TRIGGER_RUNTIME = TriggerRuntime(
    get_settings=attractor_server.get_settings,
    get_attractor_client=lambda: AttractorApiClient(
        base_url="http://attractor.internal",
        app=attractor_server.attractor_app,
    ),
)


def _pick_directory_with_osascript(prompt: str) -> Path | None:
    escaped_prompt = prompt.replace('"', '\\"')
    completed = subprocess.run(
        [
            "osascript",
            "-e",
            "try",
            "-e",
            f'POSIX path of (choose folder with prompt "{escaped_prompt}")',
            "-e",
            "on error number -128",
            "-e",
            'return "__CANCELED__"',
            "-e",
            "end try",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        message = (completed.stderr or completed.stdout or "Native macOS directory picker failed.").strip()
        raise RuntimeError(message)
    selected_path = completed.stdout.strip()
    if not selected_path or selected_path == "__CANCELED__":
        return None
    return Path(selected_path).expanduser().resolve()


def _pick_directory_with_tk(prompt: str) -> Path | None:
    try:
        import tkinter as tk
        from tkinter import filedialog
    except Exception as exc:  # pragma: no cover - platform-dependent fallback
        raise RuntimeError("Tk directory picker is unavailable.") from exc

    root = tk.Tk()
    root.withdraw()
    try:
        root.attributes("-topmost", True)
    except Exception:
        pass
    try:
        selected_path = filedialog.askdirectory(title=prompt, mustexist=True)
    finally:
        root.destroy()
    if not selected_path:
        return None
    return Path(selected_path).expanduser().resolve()


def _pick_project_directory(prompt: str = "Select Spark project directory") -> Path | None:
    picker_errors: list[str] = []
    if sys.platform == "darwin" and shutil.which("osascript"):
        try:
            return _pick_directory_with_osascript(prompt)
        except RuntimeError as exc:
            picker_errors.append(str(exc))
    try:
        return _pick_directory_with_tk(prompt)
    except RuntimeError as exc:
        picker_errors.append(str(exc))
    raise RuntimeError(picker_errors[-1] if picker_errors else "No native directory picker is available in this runtime.")


@asynccontextmanager
async def _workspace_lifespan(_: FastAPI):
    await TRIGGER_RUNTIME.start()
    try:
        yield
    finally:
        await TRIGGER_RUNTIME.stop()


@asynccontextmanager
async def _product_lifespan(_: FastAPI):
    attractor_server.initialize_attractor_runtime()
    try:
        yield
    finally:
        attractor_server.shutdown_attractor_runtime()


workspace_app = FastAPI(
    title="Spark Workspace API",
    docs_url="/docs",
    redoc_url=None,
    openapi_url="/openapi.json",
    lifespan=_workspace_lifespan,
)

app.router.lifespan_context = _product_lifespan


WORKSPACE_ROUTER = create_workspace_router(
    WorkspaceApiDependencies(
        get_settings=attractor_server.get_settings,
        get_project_chat=lambda: attractor_server.PROJECT_CHAT,
        get_attractor_client=lambda: AttractorApiClient(
            base_url="http://attractor.internal",
            app=attractor_server.attractor_app,
        ),
        resolve_project_git_branch=lambda runtime_path: pipeline_runs.resolve_project_git_branch(runtime_path),
        resolve_project_git_commit=lambda runtime_path: pipeline_runs.resolve_project_git_commit(runtime_path),
        pick_project_directory=lambda: _pick_project_directory(),
        get_trigger_runtime=lambda: TRIGGER_RUNTIME,
        default_execution_planning_flow=DEFAULT_EXECUTION_PLANNING_FLOW,
        default_execution_dispatch_flow=DEFAULT_EXECUTION_DISPATCH_FLOW,
    )
)

workspace_app.include_router(WORKSPACE_ROUTER)


app.mount("/attractor", attractor_server.attractor_app)
app.mount("/workspace", workspace_app)


@app.get("/")
async def get_ui():
    index_path = resolve_ui_index_path(attractor_server.get_settings())
    if not index_path:
        raise HTTPException(status_code=404, detail="UI index not found")
    return FileResponse(index_path)


@app.get("/assets/{asset_path:path}")
async def get_frontend_asset(asset_path: str):
    file_path = resolve_ui_asset_path(attractor_server.get_settings(), f"assets/{asset_path}")
    if not file_path:
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


@app.get("/vite.svg")
async def get_frontend_vite_icon():
    file_path = resolve_ui_asset_path(attractor_server.get_settings(), "vite.svg")
    if not file_path:
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)
