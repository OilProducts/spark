from __future__ import annotations

import asyncio
import json
import mimetypes
from dataclasses import dataclass, field
import threading
import uuid
import os
import shutil
import re
import selectors
import sys
import time
from datetime import datetime
from pathlib import Path
import subprocess
from typing import Any, Callable, Dict, List, Optional

from fastapi import FastAPI, HTTPException, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, StreamingResponse
from pydantic import AliasChoices, BaseModel, Field

from attractor.dsl import (
    canonicalize_dot,
    DotParseError,
    Diagnostic,
    DiagnosticSeverity,
    format_dot,
    normalize_graph,
    parse_dot,
    validate_graph,
)
from attractor.engine import (
    Checkpoint,
    Context,
    Outcome,
    OutcomeStatus,
    PipelineExecutor,
    load_checkpoint,
    save_checkpoint,
)
from attractor.graphviz_export import export_graphviz_artifact
from attractor.config import Settings, resolve_settings, validate_settings
from attractor.api.project_chat import (
    ProjectChatService,
    build_codex_runtime_environment,
    resolve_runtime_workspace_path,
)
from attractor.storage import (
    build_project_id,
    ensure_project_paths,
    list_project_records,
    normalize_project_path,
    read_project_record,
    read_project_paths_by_id,
    update_project_record,
)
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import CodergenBackend
from attractor.interviewer.base import Interviewer
from attractor.interviewer.models import Answer, AnswerValue, Question
from attractor.transforms import (
    GoalVariableTransform,
    ModelStylesheetTransform,
    TransformPipeline,
)


app = FastAPI()
SETTINGS_LOCK = threading.Lock()
SETTINGS = resolve_settings()
PROJECT_CHAT = ProjectChatService(SETTINGS.data_dir)
REGISTERED_TRANSFORMS: List[object] = []
_REGISTERED_TRANSFORMS_LOCK = threading.Lock()


def get_settings() -> Settings:
    with SETTINGS_LOCK:
        return SETTINGS


def configure_runtime_paths(
    *,
    data_dir: Path | str | None = None,
    runs_dir: Path | str | None = None,
    flows_dir: Path | str | None = None,
    ui_dir: Path | str | None = None,
) -> Settings:
    global SETTINGS, PROJECT_CHAT
    current = get_settings()
    _ = runs_dir
    updated = resolve_settings(
        data_dir=data_dir if data_dir is not None else current.data_dir,
        flows_dir=flows_dir if flows_dir is not None else current.flows_dir,
        ui_dir=ui_dir if ui_dir is not None else current.ui_dir,
    )
    validate_settings(updated)
    with SETTINGS_LOCK:
        SETTINGS = updated
    PROJECT_CHAT = ProjectChatService(updated.data_dir)
    return updated


def validate_runtime_paths() -> None:
    validate_settings(get_settings())


def _resolve_ui_index_path() -> Path | None:
    settings = get_settings()
    if settings.ui_dir:
        index_path = settings.ui_dir / "index.html"
        if index_path.exists():
            return index_path
    if settings.legacy_ui_index and settings.legacy_ui_index.exists():
        return settings.legacy_ui_index
    return None


def _resolve_ui_asset_path(relative_path: str) -> Path | None:
    settings = get_settings()
    if not settings.ui_dir:
        return None
    candidate = settings.ui_dir / relative_path
    if candidate.exists():
        return candidate
    return None


def register_transform(transform: object) -> None:
    with _REGISTERED_TRANSFORMS_LOCK:
        REGISTERED_TRANSFORMS.append(transform)


def clear_registered_transforms() -> None:
    with _REGISTERED_TRANSFORMS_LOCK:
        REGISTERED_TRANSFORMS.clear()


def _registered_transforms_snapshot() -> List[object]:
    with _REGISTERED_TRANSFORMS_LOCK:
        return list(REGISTERED_TRANSFORMS)


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
    project_path: str = ""
    git_branch: Optional[str] = None
    git_commit: Optional[str] = None
    spec_id: Optional[str] = None
    plan_id: Optional[str] = None
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
            "project_path": self.project_path,
            "git_branch": self.git_branch,
            "git_commit": self.git_commit,
            "spec_id": self.spec_id,
            "plan_id": self.plan_id,
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
            project_path=str(data.get("project_path", "")),
            git_branch=str(data.get("git_branch")) if data.get("git_branch") is not None else None,
            git_commit=str(data.get("git_commit")) if data.get("git_commit") is not None else None,
            spec_id=str(data.get("spec_id")) if data.get("spec_id") is not None else None,
            plan_id=str(data.get("plan_id")) if data.get("plan_id") is not None else None,
            last_error=str(data.get("last_error", "")),
            token_usage=int(data["token_usage"]) if data.get("token_usage") is not None else None,
        )


RUN_HISTORY_LOCK = threading.Lock()
PIPELINE_LIFECYCLE_PHASES = ("PARSE", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE")


def _runs_root() -> Path:
    root = get_settings().projects_dir
    root.mkdir(parents=True, exist_ok=True)
    return root


def _project_runs_dir(project_path: str) -> Optional[Path]:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        return None
    project_id = build_project_id(normalized_project_path)
    project_paths = read_project_paths_by_id(get_settings().data_dir, project_id)
    if project_paths is None:
        return None
    return project_paths.runs_dir


def _iter_run_roots(*, project_path: Optional[str] = None) -> list[Path]:
    if project_path:
        runs_dir = _project_runs_dir(project_path)
        if runs_dir is None or not runs_dir.exists():
            return []
        return sorted((path for path in runs_dir.iterdir() if path.is_dir()), key=lambda item: item.name)

    run_roots: list[Path] = []
    projects_root = _runs_root()
    if not projects_root.exists():
        return run_roots
    for runs_dir in sorted(projects_root.glob("*/runs")):
        if not runs_dir.is_dir():
            continue
        run_roots.extend(sorted((path for path in runs_dir.iterdir() if path.is_dir()), key=lambda item: item.name))
    return run_roots


def _find_run_root(run_id: str) -> Optional[Path]:
    for run_root in _iter_run_roots():
        if run_root.name == run_id:
            return run_root
    return None


def _ensure_run_root_for_project(run_id: str, project_path: str) -> Path:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        raise ValueError("Run storage requires a project path.")
    project_paths = ensure_project_paths(get_settings().data_dir, normalized_project_path)
    run_root = project_paths.runs_dir / run_id
    run_root.mkdir(parents=True, exist_ok=True)
    return run_root


def _run_root(run_id: str) -> Path:
    run_root = _find_run_root(run_id)
    if run_root is not None:
        return run_root
    return get_settings().runtime_dir / "_missing-runs" / run_id


def _resolve_start_node_id(graph) -> str:
    shape_starts = []
    for node in graph.nodes.values():
        shape_attr = node.attrs.get("shape")
        shape_value = str(shape_attr.value) if shape_attr is not None else ""
        if shape_value == "Mdiamond":
            shape_starts.append(node.node_id)

    candidates = shape_starts or [node_id for node_id in graph.nodes if node_id in {"start", "Start"}]
    if len(candidates) != 1:
        raise RuntimeError(f"Expected exactly one start node, found {len(candidates)}")
    return candidates[0]


def _graph_attr_context_seed(graph) -> Dict[str, object]:
    seeded: Dict[str, object] = {}
    for key, attr in graph.graph_attrs.items():
        value = getattr(attr, "value", "")
        if hasattr(value, "raw"):
            value = value.raw
        seeded[f"graph.{key}"] = value
    seeded.setdefault("graph.goal", "")
    return seeded


def _run_meta_path(run_id: str) -> Path:
    return _run_root(run_id) / "run.json"


def _write_run_meta(record: RunRecord) -> None:
    try:
        if record.project_path or record.working_directory:
            _ensure_run_root_for_project(record.run_id, record.project_path or record.working_directory)
        run_meta_path = _run_meta_path(record.run_id)
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


def _normalize_scope_path(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        return ""
    slash_normalized = re.sub(r"/{2,}", "/", trimmed.replace("\\", "/"))
    prefix = "/" if slash_normalized.startswith("/") else ""
    raw_body = slash_normalized[1:] if prefix else slash_normalized
    parts = [part for part in raw_body.split("/") if part]
    segments: List[str] = []
    for part in parts:
        if part == ".":
            continue
        if part == "..":
            if segments:
                segments.pop()
            continue
        segments.append(part)
    normalized_body = "/".join(segments)
    if not normalized_body and prefix:
        return prefix
    return f"{prefix}{normalized_body}"


def _path_in_scope(candidate_path: str, project_scope_path: str) -> bool:
    if not candidate_path or not project_scope_path:
        return False
    if candidate_path == project_scope_path:
        return True
    if project_scope_path == "/":
        return candidate_path.startswith("/")
    return candidate_path.startswith(f"{project_scope_path}/")


def _run_matches_project_scope(record: RunRecord, project_path: str) -> bool:
    normalized_scope = _normalize_scope_path(project_path)
    if not normalized_scope:
        return True
    candidate_paths = [
        _normalize_scope_path(record.project_path),
        _normalize_scope_path(record.working_directory),
    ]
    return any(_path_in_scope(candidate_path, normalized_scope) for candidate_path in candidate_paths)


def _record_run_start(
    run_id: str,
    flow_name: str,
    working_directory: str,
    model: str,
    spec_id: Optional[str] = None,
    plan_id: Optional[str] = None,
) -> None:
    project_path, git_branch, git_commit = _resolve_run_project_git_metadata(working_directory)
    record = RunRecord(
        run_id=run_id,
        flow_name=flow_name,
        status="running",
        result=None,
        working_directory=working_directory,
        model=model,
        started_at=datetime.utcnow().isoformat(timespec="seconds") + "Z",
        project_path=project_path,
        git_branch=git_branch,
        git_commit=git_commit,
        spec_id=spec_id,
        plan_id=plan_id,
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


RUN_LOG_TIMESTAMP_RE = re.compile(r"^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) UTC\]")


def _hydrate_run_record_from_log(record: RunRecord, run_log_path: Path) -> None:
    if not run_log_path.exists():
        return
    record.token_usage = _extract_token_usage(record.run_id)
    try:
        lines = run_log_path.read_text(encoding="utf-8", errors="ignore").splitlines()
    except Exception:
        return
    if not lines:
        return

    first_timestamp = RUN_LOG_TIMESTAMP_RE.search(lines[0])
    if first_timestamp and not record.started_at:
        record.started_at = f"{first_timestamp.group(1).replace(' ', 'T')}Z"

    log_status = None
    for line in reversed(lines):
        status_match = re.search(r"Pipeline\s+(\w+)", line)
        if status_match:
            log_status = _normalize_run_status(status_match.group(1))
            break
        if "Pipeline Aborted" in line:
            log_status = "canceled"
            break

    if log_status and record.status in {"", "unknown", "running"}:
        record.status = log_status
    if log_status and record.result is None:
        record.result = log_status
    if log_status and not record.ended_at:
        last_timestamp = RUN_LOG_TIMESTAMP_RE.search(lines[-1])
        if last_timestamp:
            record.ended_at = f"{last_timestamp.group(1).replace(' ', 'T')}Z"
    if not log_status and record.status == "unknown":
        record.status = "running"


def _ensure_known_pipeline(pipeline_id: str) -> None:
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")


def _artifact_media_type(path: Path) -> str:
    guessed, _ = mimetypes.guess_type(path.name)
    return guessed or "application/octet-stream"


def _artifact_is_viewable(*, media_type: str, path: Path) -> bool:
    if media_type.startswith("text/"):
        return True
    if media_type in {"application/json", "application/xml", "image/svg+xml"}:
        return True
    return path.suffix.lower() in {".json", ".txt", ".md", ".log", ".dot", ".yaml", ".yml", ".csv"}


def _resolve_artifact_path(run_root: Path, artifact_path: str) -> Path:
    normalized = artifact_path.strip()
    if not normalized:
        raise HTTPException(status_code=400, detail="Invalid artifact path")

    candidate = Path(normalized)
    if candidate.is_absolute():
        raise HTTPException(status_code=400, detail="Invalid artifact path")

    resolved_run_root = run_root.resolve()
    resolved_target = (resolved_run_root / candidate).resolve()
    try:
        resolved_target.relative_to(resolved_run_root)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail="Invalid artifact path") from exc
    return resolved_target


def _list_run_output_artifacts(run_root: Path) -> List[Dict[str, object]]:
    files: Dict[str, Path] = {}

    def _add_file(path: Path) -> None:
        if not path.is_file():
            return
        try:
            relative_path = path.relative_to(run_root).as_posix()
        except ValueError:
            return
        files[relative_path] = path

    _add_file(run_root / "manifest.json")
    _add_file(run_root / "checkpoint.json")

    if run_root.exists():
        for child in run_root.iterdir():
            if not child.is_dir() or child.name == "artifacts":
                continue
            _add_file(child / "prompt.md")
            _add_file(child / "response.md")
            _add_file(child / "status.json")

    artifacts_root = run_root / "artifacts"
    if artifacts_root.exists():
        for file_path in artifacts_root.rglob("*"):
            _add_file(file_path)

    entries: List[Dict[str, object]] = []
    for relative_path in sorted(files):
        absolute_path = files[relative_path]
        media_type = _artifact_media_type(absolute_path)
        entries.append(
            {
                "path": relative_path,
                "size_bytes": absolute_path.stat().st_size,
                "media_type": media_type,
                "viewable": _artifact_is_viewable(media_type=media_type, path=absolute_path),
            }
        )
    return entries


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
                project_path=working_directory,
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


async def _publish_lifecycle_phase(run_id: str, phase: str) -> None:
    await _publish_run_event(run_id, {"type": "lifecycle", "phase": phase})


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


def _read_checkpoint_progress(run_id: str) -> tuple[str, List[str]]:
    checkpoint = load_checkpoint(_run_root(run_id) / "state.json")
    if checkpoint is None:
        return "", []
    return checkpoint.current_node, list(checkpoint.completed_nodes)


def _pipeline_progress_payload(current_node: str, completed_nodes: List[str]) -> Dict[str, object]:
    return {
        "current_node": current_node,
        "completed_count": len(completed_nodes),
    }


def _pop_active_run(run_id: str) -> Optional[ActiveRun]:
    with ACTIVE_RUNS_LOCK:
        return ACTIVE_RUNS.pop(run_id, None)


class PipelineStartRequest(BaseModel):
    flow_content: str = Field(validation_alias=AliasChoices("flow_content", "dot_source"))
    working_directory: str = "./workspace"
    backend: str = "codex"
    model: Optional[str] = None
    flow_name: Optional[str] = None
    spec_id: Optional[str] = None
    plan_id: Optional[str] = None


class PreviewRequest(BaseModel):
    flow_content: str


class SaveFlowRequest(BaseModel):
    name: str
    content: str
    expect_semantic_equivalence: bool = False


class ResetRequest(BaseModel):
    working_directory: str = "./workspace"


class HumanAnswerRequest(BaseModel):
    selected_value: str


class LegacyHumanAnswerRequest(BaseModel):
    pipeline_id: str
    question_id: str
    selected_value: str


class ConversationTurnRequest(BaseModel):
    project_path: str
    message: str
    model: Optional[str] = None


class SpecEditApprovalRequest(BaseModel):
    project_path: str
    model: Optional[str] = None
    flow_source: Optional[str] = None


class SpecEditRejectionRequest(BaseModel):
    project_path: str


class ExecutionCardReviewRequest(BaseModel):
    project_path: str
    disposition: str
    message: str
    model: Optional[str] = None
    flow_source: Optional[str] = None


class ProjectRegistrationRequest(BaseModel):
    project_path: str


class ProjectStateUpdateRequest(BaseModel):
    project_path: str
    is_favorite: Optional[bool] = None
    last_accessed_at: Optional[str] = None
    active_conversation_id: Optional[str] = None


DEFAULT_FLOW = """digraph SoftwareFactory {
    start [shape=Mdiamond, label="Start"];
    setup [shape=box, prompt="Initialize project"];
    build [shape=box, prompt="Build app"];
    done [shape=Msquare, label="Done"];

    start -> setup -> build -> done;
}"""


class LocalCodexCliBackend(CodergenBackend):
    def __init__(self, working_dir: str, emit, model: Optional[str] = None):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.model = model

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> str | Outcome:
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
                env=build_codex_runtime_environment(),
                capture_output=True,
                text=True,
                timeout=timeout,
                start_new_session=True,
            )
        except FileNotFoundError:
            if not Path(self.working_dir).exists():
                failure_reason = (
                    "codex working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir}, resolved {self.working_dir}"
                )
            else:
                failure_reason = "codex executable not found on PATH"
            self.emit({"type": "log", "msg": f"[{node_id}] {failure_reason}"})
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=failure_reason)
        except subprocess.TimeoutExpired:
            failure_reason = f"timeout after {timeout}s"
            self.emit({"type": "log", "msg": f"[{node_id}] {failure_reason}"})
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=failure_reason)
        if proc.stdout.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stdout.strip()}"})
        if proc.stderr.strip():
            self.emit({"type": "log", "msg": f"[{node_id}] {proc.stderr.strip()}"})
        if proc.returncode != 0:
            failure_reason = proc.stderr.strip() or f"codex cli exited with code {proc.returncode}"
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=failure_reason)
        output = proc.stdout.strip()
        return output if output else "codex cli completed successfully"


class LocalCodexAppServerBackend(CodergenBackend):
    RUNTIME_THREAD_ID_KEY = "_attractor.runtime.thread_id"

    def __init__(self, working_dir: str, emit, model: Optional[str] = None):
        self.requested_working_dir = str(Path(working_dir).expanduser().resolve(strict=False))
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self.emit = emit
        self.model = model
        self._session_threads_by_key: dict[str, str] = {}
        self._session_threads_lock = threading.Lock()

    def _runtime_thread_key(self, context: Context) -> str:
        value = context.get(self.RUNTIME_THREAD_ID_KEY, "")
        if value is None:
            return ""
        return str(value).strip()

    def _resolve_session_thread_id(
        self,
        thread_key: str,
        start_thread: Callable[[], str | None],
    ) -> str | None:
        normalized_key = thread_key.strip()
        if not normalized_key:
            return start_thread()

        with self._session_threads_lock:
            cached = self._session_threads_by_key.get(normalized_key)
            if cached:
                return cached
            created = start_thread()
            if not created:
                return None
            self._session_threads_by_key[normalized_key] = created
            return created

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        timeout: Optional[float] = None,
    ) -> str | Outcome:
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

        def fail(reason: str) -> Outcome:
            log_line(reason)
            return Outcome(status=OutcomeStatus.FAIL, failure_reason=reason)

        try:
            proc = subprocess.Popen(
                cmd,
                cwd=self.working_dir,
                env=build_codex_runtime_environment(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                start_new_session=True,
            )
        except FileNotFoundError:
            if not Path(self.working_dir).exists():
                return fail(
                    "codex app-server working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir}, resolved {self.working_dir}"
                )
            return fail("codex app-server not found on PATH")

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

        saw_item_agent_message_delta = False

        def handle_notification(message: dict) -> None:
            nonlocal last_token_total, turn_status, turn_error, saw_item_agent_message_delta
            method = message.get("method")
            params = message.get("params") or {}
            if method == "item/agentMessage/delta":
                delta = params.get("delta") or ""
                if delta:
                    saw_item_agent_message_delta = True
                    agent_chunks.append(delta)
                return
            if method == "codex/event/agent_message_delta":
                if saw_item_agent_message_delta:
                    return
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
                return
            if method == "codex/event/task_complete":
                return

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
                return fail("app-server initialize failed")

            def start_thread() -> str | None:
                thread_params = {
                    "cwd": self.working_dir,
                    "sandbox": "danger-full-access",
                    "ephemeral": True,
                }
                if self.model:
                    thread_params["model"] = self.model
                thread_request_id = send_request("thread/start", thread_params)
                thread_response = wait_for_response(thread_request_id)
                if not thread_response or thread_response.get("error"):
                    return None
                thread = (thread_response.get("result") or {}).get("thread") or {}
                thread_uuid = thread.get("id")
                if not thread_uuid:
                    return None
                return str(thread_uuid)

            thread_key = self._runtime_thread_key(context)
            thread_uuid = self._resolve_session_thread_id(thread_key, start_thread)
            if not thread_uuid:
                return fail("app-server thread/start failed")

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
                return fail("app-server turn/start failed")

            completed = wait_for_turn_completion()
            if not completed:
                return fail("app-server turn timed out or exited early")

            if turn_status and turn_status != "completed":
                return fail(turn_error or f"app-server turn ended with status '{turn_status}'")

            agent_text = "".join(agent_chunks).strip()
            if agent_text:
                log_line(agent_text)
            command_text = "".join(command_chunks).strip()
            if command_text:
                log_line(command_text)
            if last_token_total is not None:
                log_line(f"tokens used: {last_token_total}")
            if agent_text:
                return agent_text
            if command_text:
                return command_text
            return "codex app-server completed successfully"
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


def _build_codergen_backend(
    backend_name: str,
    working_dir: str,
    emit: Callable[[dict], None],
    *,
    model: Optional[str],
) -> CodergenBackend:
    normalized = backend_name.strip().lower().replace("_", "-")
    if normalized in {"codex", "codex-app-server"}:
        return LocalCodexAppServerBackend(working_dir, emit, model=model)
    if normalized == "codex-cli":
        return LocalCodexCliBackend(working_dir, emit, model=model)
    raise ValueError(
        "Unsupported backend. Supported backends: codex, codex-app-server, codex-cli."
    )


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


@app.get("/")
async def get_ui():
    index_path = _resolve_ui_index_path()
    if not index_path:
        raise HTTPException(status_code=404, detail="UI index not found")
    return FileResponse(index_path)


@app.get("/assets/{asset_path:path}")
async def get_frontend_asset(asset_path: str):
    file_path = _resolve_ui_asset_path(f"assets/{asset_path}")
    if not file_path:
        raise HTTPException(status_code=404, detail="Asset not found")
    return FileResponse(file_path)


@app.get("/vite.svg")
async def get_frontend_vite_icon():
    file_path = _resolve_ui_asset_path("vite.svg")
    if not file_path:
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
async def list_runs(project_path: Optional[str] = None):
    records: List[RunRecord] = []
    for run_dir in _iter_run_roots():
        if not run_dir.is_dir():
            continue
        meta_path = run_dir / "run.json"
        run_log_path = run_dir / "run.log"
        record = _read_run_meta(meta_path)
        if record:
            _hydrate_run_record_from_log(record, run_log_path)
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
        _hydrate_run_record_from_log(record, run_log_path)
        records.append(record)

    def _sort_key(item: RunRecord) -> str:
        return item.started_at or item.ended_at or ""

    if project_path:
        records = [record for record in records if _run_matches_project_scope(record, project_path)]

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

    def _all_attrs_payload(attrs: Dict[str, object]) -> Dict[str, object]:
        return {key: _attr_value(attrs, key) for key in attrs}

    def _merge_extension_attrs(payload: Dict[str, object], attrs: Dict[str, object]) -> Dict[str, object]:
        merged = dict(payload)
        for key, value in _all_attrs_payload(attrs).items():
            if key not in merged:
                merged[key] = value
        return merged

    def _defaults_payload(scope) -> dict:
        return {
            "node": _all_attrs_payload(scope.node),
            "edge": _all_attrs_payload(scope.edge),
        }

    def _subgraph_payload(scope) -> dict:
        return {
            "id": scope.id,
            "attrs": _all_attrs_payload(scope.attrs),
            "node_ids": list(scope.node_ids),
            "defaults": _defaults_payload(scope.defaults),
            "subgraphs": [_subgraph_payload(child) for child in scope.subgraphs],
        }

    return {
        "nodes": [
            _merge_extension_attrs({
                "id": n.node_id,
                "label": _attr_value(n.attrs, "label", n.node_id),
                "shape": _attr_value(n.attrs, "shape"),
                "prompt": _attr_value(n.attrs, "prompt"),
                "tool_command": _attr_value(n.attrs, "tool_command"),
                "tool_hooks.pre": _attr_value(n.attrs, "tool_hooks.pre"),
                "tool_hooks.post": _attr_value(n.attrs, "tool_hooks.post"),
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
                "manager.poll_interval": _attr_value(n.attrs, "manager.poll_interval"),
                "manager.max_cycles": _attr_value(n.attrs, "manager.max_cycles"),
                "manager.stop_condition": _attr_value(n.attrs, "manager.stop_condition"),
                "manager.actions": _attr_value(n.attrs, "manager.actions"),
                "human.default_choice": _attr_value(n.attrs, "human.default_choice"),
            }, n.attrs)
            for n in graph.nodes.values()
        ],
        "graph_attrs": _merge_extension_attrs({
            "goal": _attr_value(graph.graph_attrs, "goal"),
            "label": _attr_value(graph.graph_attrs, "label", ""),
            "model_stylesheet": _attr_value(graph.graph_attrs, "model_stylesheet"),
            "default_max_retry": _attr_value(graph.graph_attrs, "default_max_retry"),
            "retry_target": _attr_value(graph.graph_attrs, "retry_target"),
            "fallback_retry_target": _attr_value(graph.graph_attrs, "fallback_retry_target"),
            "default_fidelity": _attr_value(graph.graph_attrs, "default_fidelity"),
            "stack.child_dotfile": _attr_value(graph.graph_attrs, "stack.child_dotfile"),
            "stack.child_workdir": _attr_value(graph.graph_attrs, "stack.child_workdir"),
            "tool_hooks.pre": _attr_value(graph.graph_attrs, "tool_hooks.pre"),
            "tool_hooks.post": _attr_value(graph.graph_attrs, "tool_hooks.post"),
            "ui_default_llm_model": _attr_value(graph.graph_attrs, "ui_default_llm_model"),
            "ui_default_llm_provider": _attr_value(graph.graph_attrs, "ui_default_llm_provider"),
            "ui_default_reasoning_effort": _attr_value(graph.graph_attrs, "ui_default_reasoning_effort"),
        }, graph.graph_attrs),
        "edges": [
            _merge_extension_attrs({
                "from": e.source,
                "to": e.target,
                "label": _attr_value(e.attrs, "label"),
                "condition": _attr_value(e.attrs, "condition"),
                "weight": _attr_value(e.attrs, "weight"),
                "fidelity": _attr_value(e.attrs, "fidelity"),
                "thread_id": _attr_value(e.attrs, "thread_id"),
                "loop_restart": _attr_value(e.attrs, "loop_restart"),
            }, e.attrs)
            for e in graph.edges
        ],
        "defaults": _defaults_payload(graph.defaults),
        "subgraphs": [_subgraph_payload(subgraph) for subgraph in graph.subgraphs],
    }


def _diagnostic_payload(diagnostic: Diagnostic) -> dict:
    payload = {
        "rule": diagnostic.rule_id,
        "rule_id": diagnostic.rule_id,
        "severity": diagnostic.severity.value,
        "message": diagnostic.message,
        "line": diagnostic.line,
        "node": diagnostic.node_id,
        "node_id": diagnostic.node_id,
    }
    if diagnostic.edge is not None:
        payload["edge"] = list(diagnostic.edge)
    if diagnostic.fix is not None:
        payload["fix"] = diagnostic.fix
    return payload


def _build_transform_pipeline() -> TransformPipeline:
    pipeline = TransformPipeline()
    pipeline.register(GoalVariableTransform())
    pipeline.register(ModelStylesheetTransform())
    for transform in _registered_transforms_snapshot():
        pipeline.register(transform)
    return pipeline


@app.post("/preview")
async def preview_pipeline(req: PreviewRequest):
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
        }
        return {
            "status": "parse_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    graph = _build_transform_pipeline().apply(graph)

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
    run_id = uuid.uuid4().hex
    await _publish_lifecycle_phase(run_id, PIPELINE_LIFECYCLE_PHASES[0])
    try:
        graph = parse_dot(req.flow_content)
    except DotParseError as exc:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = str(exc)
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
        }
        if RUNTIME.last_run_id:
            await _publish_run_event(RUNTIME.last_run_id, {"type": "log", "msg": f"❌ Parse error: {exc}"})
        return {
            "status": "validation_error",
            "error": str(exc),
            "diagnostics": [parse_diag],
            "errors": [parse_diag],
        }

    graph = _build_transform_pipeline().apply(graph)

    await _publish_lifecycle_phase(run_id, PIPELINE_LIFECYCLE_PHASES[1])
    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    diagnostic_payloads = [_diagnostic_payload(d) for d in diagnostics]
    error_payloads = [_diagnostic_payload(d) for d in errors]
    if errors:
        RUNTIME.status = "validation_error"
        RUNTIME.last_error = errors[0].message
        return {
            "status": "validation_error",
            "diagnostics": diagnostic_payloads,
            "errors": error_payloads,
        }

    await _publish_lifecycle_phase(run_id, PIPELINE_LIFECYCLE_PHASES[2])

    os.makedirs(req.working_directory, exist_ok=True)
    working_dir = str(Path(req.working_directory).resolve())
    selected_model = (req.model or "").strip()
    flow_name = (req.flow_name or "").strip()
    display_model = selected_model or "codex default (config/profile)"

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

    try:
        backend = _build_codergen_backend(
            req.backend,
            working_dir,
            emit,
            model=selected_model or None,
        )
    except ValueError as exc:
        return {
            "status": "validation_error",
            "error": str(exc),
        }

    interviewer: Interviewer = WebInterviewer(HUMAN_BROKER, emit, flow_name, run_id)

    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=interviewer,
    )
    runner = BroadcastingRunner(HandlerRunner(graph, registry), emit)

    run_root = _ensure_run_root_for_project(run_id, working_dir)
    checkpoint_file = str(run_root / "state.json")
    logs_root = str(run_root / "logs")
    # NOTE: This artifact render intentionally uses the submitted DOT source.
    # It does not reflect transform/normalization changes applied to `graph`.
    # If post-transform fidelity is required, render from a serialized `graph` instead.
    graphviz_export = export_graphviz_artifact(req.flow_content, run_root)

    context = Context(values=_graph_attr_context_seed(graph))
    run_root.mkdir(parents=True, exist_ok=True)
    Path(logs_root).mkdir(parents=True, exist_ok=True)
    save_checkpoint(
        Path(checkpoint_file),
        Checkpoint(
            current_node=_resolve_start_node_id(graph),
            completed_nodes=[],
            context=dict(context.values),
            retry_counts={},
        ),
    )

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

    _record_run_start(
        run_id,
        flow_name,
        working_dir,
        display_model,
        spec_id=(req.spec_id or "").strip() or None,
        plan_id=(req.plan_id or "").strip() or None,
    )

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
            await _publish_lifecycle_phase(run_id, PIPELINE_LIFECYCLE_PHASES[3])
            active = _get_active_run(run_id)
            control = active.control if active else ExecutionControl()
            executor = PipelineExecutor(
                graph,
                runner,
                logs_root=logs_root,
                checkpoint_file=checkpoint_file,
                control=control.poll,
                on_event=emit,
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
            await _publish_lifecycle_phase(run_id, PIPELINE_LIFECYCLE_PHASES[4])
            _pop_active_run(run_id)

    asyncio.create_task(_run())
    return {
        "status": "started",
        "pipeline_id": run_id,
        "run_id": run_id,
        "working_directory": working_dir,
        "model": display_model,
        "diagnostics": diagnostic_payloads,
        "errors": error_payloads,
        "graph_dot_path": str(graphviz_export.dot_path),
        "graph_render_path": str(graphviz_export.rendered_path) if graphviz_export.rendered_path else None,
    }


@app.post("/pipelines")
async def create_pipeline(req: PipelineStartRequest):
    return await _start_pipeline(req)


@app.post("/run")
async def run_pipeline(req: PipelineStartRequest):
    return await _start_pipeline(req)


@app.get("/pipelines/{pipeline_id}")
async def get_pipeline(pipeline_id: str):
    checkpoint_current_node, checkpoint_completed_nodes = _read_checkpoint_progress(pipeline_id)
    active = _get_active_run(pipeline_id)
    if active:
        completed_nodes = list(active.completed_nodes) if active.completed_nodes else checkpoint_completed_nodes
        return {
            "pipeline_id": pipeline_id,
            "status": active.status,
            "flow_name": active.flow_name,
            "working_directory": active.working_directory,
            "model": active.model,
            "last_error": active.last_error,
            "completed_nodes": completed_nodes,
            "progress": _pipeline_progress_payload(checkpoint_current_node, completed_nodes),
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
        "completed_nodes": checkpoint_completed_nodes,
        "progress": _pipeline_progress_payload(checkpoint_current_node, checkpoint_completed_nodes),
        "started_at": record.started_at,
        "ended_at": record.ended_at,
        "result": record.result,
    }


@app.get("/pipelines/{pipeline_id}/checkpoint")
async def get_pipeline_checkpoint(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)

    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=404, detail="Checkpoint unavailable")

    return {
        "pipeline_id": pipeline_id,
        "checkpoint": checkpoint.to_dict(),
    }


@app.get("/pipelines/{pipeline_id}/context")
async def get_pipeline_context(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)

    checkpoint = load_checkpoint(_run_root(pipeline_id) / "state.json")
    if checkpoint is None:
        raise HTTPException(status_code=404, detail="Context unavailable")

    return {
        "pipeline_id": pipeline_id,
        "context": dict(checkpoint.context),
    }


@app.get("/pipelines/{pipeline_id}/artifacts")
async def list_pipeline_artifacts(pipeline_id: str):
    _ensure_known_pipeline(pipeline_id)
    run_root = _run_root(pipeline_id)
    return {
        "pipeline_id": pipeline_id,
        "artifacts": _list_run_output_artifacts(run_root),
    }


@app.get("/pipelines/{pipeline_id}/artifacts/{artifact_path:path}")
async def get_pipeline_artifact_file(pipeline_id: str, artifact_path: str, download: bool = False):
    _ensure_known_pipeline(pipeline_id)
    run_root = _run_root(pipeline_id)
    resolved_artifact_path = _resolve_artifact_path(run_root, artifact_path)
    if not resolved_artifact_path.is_file():
        raise HTTPException(status_code=404, detail="Artifact not found")

    media_type = _artifact_media_type(resolved_artifact_path)
    if download:
        return FileResponse(
            resolved_artifact_path,
            media_type=media_type,
            filename=resolved_artifact_path.name,
        )
    return FileResponse(resolved_artifact_path, media_type=media_type)


@app.get("/pipelines/{pipeline_id}/events")
async def pipeline_events(pipeline_id: str, request: Request):
    active = _get_active_run(pipeline_id)
    existing = _read_run_meta(_run_meta_path(pipeline_id))
    if not active and not existing and not EVENT_HUB.history(pipeline_id):
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    queue, history = EVENT_HUB.subscribe_with_history(pipeline_id)

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

    return StreamingResponse(
        stream(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    )


@app.post("/pipelines/{pipeline_id}/cancel")
async def cancel_pipeline(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active:
        if not _read_run_meta(_run_meta_path(pipeline_id)):
            raise HTTPException(status_code=404, detail="Unknown pipeline")
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


@app.get("/pipelines/{pipeline_id}/graph")
async def get_pipeline_graph(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")

    graph_svg_path = _run_root(pipeline_id) / "artifacts" / "graphviz" / "pipeline.svg"
    if not graph_svg_path.exists():
        raise HTTPException(status_code=404, detail="Graph visualization unavailable")

    return FileResponse(graph_svg_path, media_type="image/svg+xml")


@app.get("/pipelines/{pipeline_id}/questions")
async def list_pipeline_questions(pipeline_id: str):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    return {"questions": HUMAN_BROKER.list_for_run(pipeline_id)}


@app.post("/pipelines/{pipeline_id}/questions/{question_id}/answer")
async def submit_pipeline_answer(pipeline_id: str, question_id: str, req: HumanAnswerRequest):
    active = _get_active_run(pipeline_id)
    if not active and not _read_run_meta(_run_meta_path(pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")
    ok = HUMAN_BROKER.answer(pipeline_id, question_id, req.selected_value)
    if not ok:
        raise HTTPException(status_code=404, detail="Unknown question for pipeline")
    return {"status": "accepted", "pipeline_id": pipeline_id, "question_id": question_id}


@app.post("/answer")
async def answer_pipeline(req: LegacyHumanAnswerRequest):
    return await submit_pipeline_answer(
        req.pipeline_id,
        req.question_id,
        HumanAnswerRequest(selected_value=req.selected_value),
    )


@app.post("/reset")
async def reset_checkpoint(req: ResetRequest):
    projects_root = get_settings().projects_dir
    if projects_root.exists():
        for runs_dir in projects_root.glob("*/runs"):
            shutil.rmtree(runs_dir, ignore_errors=True)
    return {"status": "reset"}


def _resolve_project_git_branch(directory_path: Path) -> Optional[str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(directory_path), "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None

    branch = completed.stdout.strip()
    return branch or None


def _resolve_project_git_commit(directory_path: Path) -> Optional[str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(directory_path), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None

    commit = completed.stdout.strip()
    return commit or None


def _resolve_run_project_git_metadata(working_directory: str) -> tuple[str, Optional[str], Optional[str]]:
    normalized_working_dir = resolve_runtime_workspace_path(working_directory)
    try:
        completed = subprocess.run(
            ["git", "-C", normalized_working_dir, "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return normalized_working_dir, None, None

    project_path = completed.stdout.strip() or normalized_working_dir
    project_directory = Path(project_path)
    return (
        project_path,
        _resolve_project_git_branch(project_directory),
        _resolve_project_git_commit(project_directory),
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


def _pick_project_directory(prompt: str = "Select Spark Spawn project directory") -> Path | None:
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


def _run_project_chat_codex_prompt(project_path: str, prompt: str, model: Optional[str]) -> str:
    backend = LocalCodexAppServerBackend(
        project_path,
        lambda _message: None,
        model=(model or "").strip() or None,
    )
    result = backend.run(
        "project_chat",
        prompt,
        Context(values={}),
        timeout=300,
    )
    if isinstance(result, Outcome):
        raise RuntimeError(result.failure_reason or "Codex app-server chat run failed")
    return str(result)


@app.get("/api/conversations/{conversation_id}")
async def get_project_conversation(conversation_id: str, project_path: Optional[str] = None):
    try:
        return await asyncio.to_thread(PROJECT_CHAT.get_snapshot, conversation_id, project_path)
    except FileNotFoundError as exc:
        raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.delete("/api/conversations/{conversation_id}")
async def delete_project_conversation(conversation_id: str, project_path: str):
    try:
        return await asyncio.to_thread(PROJECT_CHAT.delete_conversation, conversation_id, project_path)
    except FileNotFoundError as exc:
        raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


def _serialize_project_record(project) -> dict[str, object]:
    return {
        "project_id": project.project_id,
        "project_path": project.project_path,
        "display_name": project.display_name,
        "created_at": project.created_at,
        "last_opened_at": project.last_opened_at,
        "last_accessed_at": project.last_accessed_at,
        "is_favorite": project.is_favorite,
        "active_conversation_id": project.active_conversation_id,
    }


@app.get("/api/projects")
async def list_projects():
    projects = await asyncio.to_thread(list_project_records, get_settings().data_dir)
    return [_serialize_project_record(project) for project in projects]


@app.post("/api/projects/register")
async def register_project(req: ProjectRegistrationRequest):
    normalized_project_path = normalize_project_path(req.project_path)
    if not normalized_project_path:
        raise HTTPException(status_code=400, detail="Project path is required.")
    try:
        project = await asyncio.to_thread(read_project_record, get_settings().data_dir, normalized_project_path)
        if project is None:
            raise ValueError("Unable to register project.")
        return _serialize_project_record(project)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.patch("/api/projects/state")
async def update_project_state(req: ProjectStateUpdateRequest):
    normalized_project_path = normalize_project_path(req.project_path)
    if not normalized_project_path:
        raise HTTPException(status_code=400, detail="Project path is required.")
    try:
        project = await asyncio.to_thread(
            update_project_record,
            get_settings().data_dir,
            normalized_project_path,
            last_accessed_at=req.last_accessed_at,
            is_favorite=req.is_favorite,
            active_conversation_id=req.active_conversation_id,
        )
        return _serialize_project_record(project)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.get("/api/projects/conversations")
async def list_project_conversations(project_path: str):
    try:
        return await asyncio.to_thread(PROJECT_CHAT.list_conversations, project_path)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.get("/api/conversations/{conversation_id}/events")
async def project_conversation_events(conversation_id: str, request: Request, project_path: Optional[str] = None):
    try:
        snapshot = await asyncio.to_thread(PROJECT_CHAT.get_snapshot, conversation_id, project_path)
    except FileNotFoundError as exc:
        raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc

    queue = PROJECT_CHAT.events().subscribe(conversation_id)

    async def stream():
        try:
            yield f"data: {json.dumps({'type': 'conversation_snapshot', 'state': snapshot})}\n\n"
            while True:
                if await request.is_disconnected():
                    break
                try:
                    event = await asyncio.wait_for(queue.get(), timeout=15.0)
                    yield f"data: {json.dumps(event)}\n\n"
                except asyncio.TimeoutError:
                    yield ": keepalive\n\n"
        finally:
            PROJECT_CHAT.events().unsubscribe(conversation_id, queue)

    return StreamingResponse(
        stream(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    )


@app.post("/api/conversations/{conversation_id}/turns")
async def send_project_conversation_turn(conversation_id: str, req: ConversationTurnRequest):
    loop = asyncio.get_running_loop()

    def publish_progress_event(event: dict[str, Any]) -> None:
        asyncio.run_coroutine_threadsafe(
            PROJECT_CHAT.events().publish(
                conversation_id,
                event,
            ),
            loop,
        )

    try:
        snapshot = await asyncio.to_thread(
            PROJECT_CHAT.start_turn,
            conversation_id,
            req.project_path,
            req.message,
            req.model,
            publish_progress_event,
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except RuntimeError as exc:
        raise HTTPException(status_code=503, detail=str(exc)) from exc
    return snapshot


@app.post("/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/approve")
async def approve_project_spec_edit_proposal(
    conversation_id: str,
    proposal_id: str,
    req: SpecEditApprovalRequest,
):
    try:
        snapshot, proposal = await asyncio.to_thread(
            PROJECT_CHAT.approve_spec_edit,
            conversation_id,
            req.project_path,
            proposal_id,
        )
        workflow_run_id = f"workflow-{uuid.uuid4().hex[:12]}"
        snapshot = await asyncio.to_thread(
            PROJECT_CHAT.mark_execution_workflow_started,
            conversation_id,
            workflow_run_id,
            req.flow_source,
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except subprocess.CalledProcessError as exc:
        detail = exc.stderr.strip() if exc.stderr else str(exc)
        raise HTTPException(status_code=500, detail=f"Failed to commit approved spec edit: {detail}") from exc

    await PROJECT_CHAT.publish_snapshot(conversation_id)
    asyncio.create_task(
        PROJECT_CHAT.run_execution_workflow(
            conversation_id,
            proposal.id,
            req.model,
            req.flow_source,
            None,
            workflow_run_id,
            lambda prompt, model: _run_project_chat_codex_prompt(req.project_path, prompt, model),
        )
    )
    return snapshot


@app.post("/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/reject")
async def reject_project_spec_edit_proposal(
    conversation_id: str,
    proposal_id: str,
    req: SpecEditRejectionRequest,
):
    try:
        snapshot = await asyncio.to_thread(
            PROJECT_CHAT.reject_spec_edit,
            conversation_id,
            req.project_path,
            proposal_id,
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    await PROJECT_CHAT.publish_snapshot(conversation_id)
    return snapshot


@app.post("/api/conversations/{conversation_id}/execution-cards/{execution_card_id}/review")
async def review_project_execution_card(
    conversation_id: str,
    execution_card_id: str,
    req: ExecutionCardReviewRequest,
):
    if req.disposition not in {"approved", "rejected", "revision_requested"}:
        raise HTTPException(status_code=400, detail="Execution card disposition must be approved, rejected, or revision_requested.")
    try:
        snapshot, execution_card, proposal_id, workflow_run_id = await asyncio.to_thread(
            PROJECT_CHAT.review_execution_card,
            conversation_id,
            req.project_path,
            execution_card_id,
            req.disposition,
            req.message,
            req.flow_source,
        )
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    await PROJECT_CHAT.publish_snapshot(conversation_id)
    if req.disposition != "approved" and proposal_id and workflow_run_id:
        asyncio.create_task(
            PROJECT_CHAT.run_execution_workflow(
                conversation_id,
                proposal_id,
                req.model,
                req.flow_source or execution_card.flow_source,
                req.message,
                workflow_run_id,
                lambda prompt, model: _run_project_chat_codex_prompt(req.project_path, prompt, model),
            )
        )
    return snapshot


@app.get("/api/projects/metadata")
async def get_project_metadata(directory: str):
    requested_path = directory.strip()
    if not requested_path:
        raise HTTPException(status_code=400, detail="Project directory path is required.")

    project_path = Path(requested_path).expanduser()
    if not project_path.is_absolute():
        raise HTTPException(status_code=400, detail="Project directory path must be absolute.")

    normalized_path = project_path.resolve(strict=False)
    runtime_path = Path(resolve_runtime_workspace_path(str(normalized_path)))
    return {
        "name": normalized_path.name or str(normalized_path),
        "directory": str(normalized_path),
        "branch": _resolve_project_git_branch(runtime_path),
        "commit": _resolve_project_git_commit(runtime_path),
    }


@app.post("/api/projects/pick-directory")
async def pick_project_directory():
    try:
        selected_directory = await asyncio.to_thread(_pick_project_directory)
    except RuntimeError as exc:
        raise HTTPException(status_code=503, detail=str(exc)) from exc
    if selected_directory is None:
        return {"status": "canceled"}
    if not selected_directory.is_absolute():
        raise HTTPException(status_code=500, detail="Directory picker returned a non-absolute path.")
    return {
        "status": "selected",
        "directory_path": str(selected_directory),
    }


@app.get("/api/flows")
async def list_flows():
    flows_dir = _flows_dir()
    return [f.name for f in flows_dir.glob("*.dot")]


def _flows_dir() -> Path:
    flows_dir = get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    return flows_dir


def _resolve_flow_path(flow_name: str) -> Path:
    raw_name = flow_name.strip()
    if not raw_name:
        raise HTTPException(status_code=400, detail="Flow name is required.")

    # Keep flow paths scoped to the project `flows/` directory.
    candidate = Path(raw_name)
    if candidate.is_absolute() or ".." in candidate.parts or len(candidate.parts) != 1:
        raise HTTPException(status_code=400, detail="Flow name must be a single file name.")

    normalized_name = candidate.name
    if not normalized_name.endswith(".dot"):
        normalized_name = f"{normalized_name}.dot"

    return _flows_dir() / normalized_name


@app.get("/api/flows/{name}")
async def get_flow(name: str):
    flow_path = _resolve_flow_path(name)
    if not flow_path.exists():
        raise HTTPException(status_code=404, detail="Flow not found.")
    return {"name": flow_path.name, "content": flow_path.read_text(encoding="utf-8")}


def _semantic_signature(dot_content: str) -> str:
    graph = _build_transform_pipeline().apply(parse_dot(dot_content))
    normalized = normalize_graph(graph)
    normalized.graph_id = "__semantic__"
    return format_dot(normalized)


@app.post("/api/flows")
async def save_flow(req: SaveFlowRequest):
    canonical_content: str
    try:
        graph = parse_dot(req.content)
        canonical_content = canonicalize_dot(req.content)
    except DotParseError as exc:
        parse_diag = {
            "rule": "parse_error",
            "rule_id": "parse_error",
            "severity": DiagnosticSeverity.ERROR.value,
            "message": str(exc),
            "line": getattr(exc, "line", 0),
            "node": None,
            "node_id": None,
        }
        raise HTTPException(
            status_code=422,
            detail={
                "status": "parse_error",
                "error": f"invalid DOT: {exc}",
                "diagnostics": [parse_diag],
                "errors": [parse_diag],
            },
        ) from exc

    graph = _build_transform_pipeline().apply(graph)
    diagnostics = validate_graph(graph)
    errors = [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]
    if errors:
        diagnostic_payloads = [_diagnostic_payload(d) for d in diagnostics]
        error_payloads = [_diagnostic_payload(d) for d in errors]
        raise HTTPException(
            status_code=422,
            detail={
                "status": "validation_error",
                "error": "validation errors prevent saving this flow",
                "diagnostics": diagnostic_payloads,
                "errors": error_payloads,
            },
        )

    flow_path = _resolve_flow_path(req.name)

    semantic_equivalent_to_existing: bool | None = None
    if flow_path.exists():
        try:
            semantic_equivalent_to_existing = _semantic_signature(flow_path.read_text()) == _semantic_signature(req.content)
        except DotParseError:
            semantic_equivalent_to_existing = None

        if req.expect_semantic_equivalence and semantic_equivalent_to_existing is False:
            raise HTTPException(
                status_code=409,
                detail={
                    "status": "semantic_mismatch",
                    "error": "semantic equivalence check failed: output DOT would change flow behavior",
                },
            )

    flow_path.write_text(canonical_content, encoding="utf-8")
    response: Dict[str, object] = {"status": "saved", "name": flow_path.name}
    if semantic_equivalent_to_existing is not None:
        response["semantic_equivalent_to_existing"] = semantic_equivalent_to_existing
    return response


@app.delete("/api/flows/{flow_name}")
async def delete_flow(flow_name: str):
    filepath = _resolve_flow_path(flow_name)
    if filepath.exists():
        filepath.unlink()
        return {"status": "deleted"}
    raise HTTPException(status_code=404, detail="Flow not found.")
