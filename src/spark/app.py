from __future__ import annotations

from contextlib import asynccontextmanager
import threading
from pathlib import Path

from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse

import attractor.api.pipeline_runs as pipeline_runs
import attractor.api.server as attractor_server
from spark.chat.service import ProjectChatService
from spark.settings import (
    SparkSettings,
    resolve_settings as resolve_spark_settings,
    validate_settings as validate_spark_settings,
)
from spark.ui import resolve_ui_asset_path, resolve_ui_index_path
from spark.workspace.api import WorkspaceApiDependencies, create_workspace_router
from spark.workspace.attractor_client import AttractorApiClient
from spark.workspace.triggers import TriggerRuntime

app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)
_UNSET = object()
_SETTINGS_LOCK = threading.Lock()
_SETTINGS = resolve_spark_settings()
_PROJECT_CHAT_LOCK = threading.Lock()
_PROJECT_CHAT: ProjectChatService | None = None
_PROJECT_CHAT_RUNTIME_KEY: tuple[Path, Path] | None = None


def _apply_settings(settings: SparkSettings) -> SparkSettings:
    global _SETTINGS
    validate_spark_settings(settings)
    attractor_server.configure_runtime_paths(
        runtime_dir=settings.runtime_dir,
        runs_dir=settings.runs_dir,
        flows_dir=settings.flows_dir,
        config_dir=settings.config_dir,
    )
    with _SETTINGS_LOCK:
        _SETTINGS = settings
    return settings


def get_settings() -> SparkSettings:
    with _SETTINGS_LOCK:
        return _SETTINGS


def configure_settings(
    *,
    data_dir: Path | str | None | object = _UNSET,
    runs_dir: Path | str | None | object = _UNSET,
    flows_dir: Path | str | None | object = _UNSET,
    ui_dir: Path | str | None | object = _UNSET,
) -> SparkSettings:
    current = get_settings()
    data_dir_overridden = data_dir is not _UNSET
    updated = resolve_spark_settings(
        data_dir=current.data_dir if data_dir is _UNSET else data_dir,
        runs_dir=current.runs_dir if runs_dir is _UNSET and not data_dir_overridden else None if runs_dir is _UNSET else runs_dir,
        flows_dir=current.flows_dir if flows_dir is _UNSET and not data_dir_overridden else None if flows_dir is _UNSET else flows_dir,
        ui_dir=current.ui_dir if ui_dir is _UNSET else ui_dir,
    )
    return _apply_settings(updated)


def validate_settings() -> None:
    validate_spark_settings(get_settings())
    attractor_server.validate_runtime_paths()


TRIGGER_RUNTIME = TriggerRuntime(
    get_settings=get_settings,
    get_attractor_client=lambda: AttractorApiClient(
        base_url="http://attractor.internal",
        app=attractor_server.attractor_app,
    ),
)


def get_project_chat() -> ProjectChatService:
    global _PROJECT_CHAT, _PROJECT_CHAT_RUNTIME_KEY
    settings = get_settings()
    runtime_key = (settings.data_dir, settings.flows_dir)
    with _PROJECT_CHAT_LOCK:
        if _PROJECT_CHAT is None or _PROJECT_CHAT_RUNTIME_KEY != runtime_key:
            _PROJECT_CHAT = ProjectChatService(settings.data_dir, flows_dir=settings.flows_dir)
            _PROJECT_CHAT_RUNTIME_KEY = runtime_key
        return _PROJECT_CHAT


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
        get_settings=get_settings,
        get_project_chat=get_project_chat,
        get_attractor_client=lambda: AttractorApiClient(
            base_url="http://attractor.internal",
            app=attractor_server.attractor_app,
        ),
        resolve_project_git_branch=lambda runtime_path: pipeline_runs.resolve_project_git_branch(runtime_path),
        resolve_project_git_commit=lambda runtime_path: pipeline_runs.resolve_project_git_commit(runtime_path),
        get_trigger_runtime=lambda: TRIGGER_RUNTIME,
    )
)

workspace_app.include_router(WORKSPACE_ROUTER)


app.mount("/attractor", attractor_server.attractor_app)
app.mount("/workspace", workspace_app)


@app.get("/")
async def get_ui():
    index_path = resolve_ui_index_path(get_settings())
    if not index_path:
        raise HTTPException(status_code=404, detail="UI index not found")
    return FileResponse(index_path)


@app.get("/assets/{asset_path:path}")
async def get_frontend_asset(asset_path: str):
    file_path = resolve_ui_asset_path(get_settings(), f"assets/{asset_path}")
    if not file_path:
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


@app.get("/vite.svg")
async def get_frontend_vite_icon():
    file_path = resolve_ui_asset_path(get_settings(), "vite.svg")
    if not file_path:
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


_apply_settings(_SETTINGS)
