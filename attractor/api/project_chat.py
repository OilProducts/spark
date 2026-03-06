from __future__ import annotations

import asyncio
import json
import os
import re
import selectors
import shutil
import subprocess
import tempfile
import threading
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional


CHAT_RUNTIME_THREAD_KEY = "_attractor.runtime.thread_id"
RUNTIME_REPO_ROOT = Path(__file__).resolve().parents[2]


def _iso_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "project"


def _normalize_project_path(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        return ""
    return str(Path(trimmed).expanduser().resolve(strict=False))


def _extract_json_object(raw: str) -> dict[str, Any]:
    text = raw.strip()
    if not text:
        raise ValueError("Expected non-empty JSON response from Codex.")
    fenced = re.search(r"```(?:json)?\s*(\{.*\})\s*```", text, flags=re.DOTALL)
    candidate = fenced.group(1) if fenced else text
    if not candidate.lstrip().startswith("{"):
        start = candidate.find("{")
        end = candidate.rfind("}")
        if start >= 0 and end > start:
            candidate = candidate[start : end + 1]
    parsed = json.loads(candidate)
    if not isinstance(parsed, dict):
        raise ValueError("Expected top-level JSON object from Codex.")
    return parsed


def resolve_runtime_workspace_path(value: str) -> str:
    normalized = _normalize_project_path(value)
    if not normalized:
        return ""

    requested_path = Path(normalized)
    if requested_path.exists():
        return str(requested_path)

    runtime_roots: list[Path] = []
    host_root_override = os.environ.get("ATTRACTOR_HOST_REPO_ROOT", "").strip()
    runtime_root_override = os.environ.get("ATTRACTOR_RUNTIME_REPO_ROOT", "").strip()
    host_root = Path(host_root_override).expanduser().resolve(strict=False) if host_root_override else None
    if runtime_root_override:
        runtime_roots.append(Path(runtime_root_override).expanduser().resolve(strict=False))
    runtime_roots.append(RUNTIME_REPO_ROOT)

    requested_parts = requested_path.parts
    for runtime_root in runtime_roots:
        if not runtime_root.exists():
            continue
        if host_root is not None:
            try:
                relative_to_host_root = requested_path.relative_to(host_root)
            except ValueError:
                relative_to_host_root = None
            if relative_to_host_root is not None:
                candidate = runtime_root / relative_to_host_root
                if candidate.exists():
                    return str(candidate.resolve(strict=False))
            host_root_matching_indexes = [index for index, part in enumerate(requested_parts) if part == host_root.name]
            for index in reversed(host_root_matching_indexes):
                candidate = runtime_root.joinpath(*requested_parts[index + 1 :])
                if candidate.exists():
                    return str(candidate.resolve(strict=False))
        matching_indexes = [index for index, part in enumerate(requested_parts) if part == runtime_root.name]
        for index in reversed(matching_indexes):
            candidate = runtime_root.joinpath(*requested_parts[index + 1 :])
            if candidate.exists():
                return str(candidate.resolve(strict=False))

    return str(requested_path)


def build_codex_runtime_environment() -> dict[str, str]:
    env = os.environ.copy()
    configured_runtime_root = Path(env.get("ATTRACTOR_CODEX_RUNTIME_ROOT", "/codex-runtime")).expanduser()
    runtime_root = configured_runtime_root
    try:
        runtime_root.mkdir(parents=True, exist_ok=True)
    except OSError:
        runtime_root = Path(tempfile.gettempdir()) / "sparkspawn-codex-runtime"
    codex_home = Path(env.get("CODEX_HOME", str(runtime_root / ".codex"))).expanduser()
    xdg_config_home = Path(env.get("XDG_CONFIG_HOME", str(runtime_root / ".config"))).expanduser()
    xdg_data_home = Path(env.get("XDG_DATA_HOME", str(runtime_root / ".local/share"))).expanduser()
    for directory in (runtime_root, codex_home, xdg_config_home, xdg_data_home):
        directory.mkdir(parents=True, exist_ok=True)

    seed_dir = Path(env.get("ATTRACTOR_CODEX_SEED_DIR", "/codex-seed")).expanduser()
    for file_name in ("auth.json", "config.toml"):
        source = seed_dir / file_name
        if not source.exists():
            continue
        destination = codex_home / file_name
        if destination.exists():
            try:
                if source.read_bytes() == destination.read_bytes():
                    continue
            except OSError:
                pass
        shutil.copy2(source, destination)

    env.update(
        {
            "HOME": str(runtime_root),
            "CODEX_HOME": str(codex_home),
            "XDG_CONFIG_HOME": str(xdg_config_home),
            "XDG_DATA_HOME": str(xdg_data_home),
        }
    )
    return env


@dataclass
class ConversationTurn:
    id: str
    role: str
    content: str
    timestamp: str
    kind: str = "message"
    artifact_id: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "role": self.role,
            "content": self.content,
            "timestamp": self.timestamp,
            "kind": self.kind,
        }
        if self.artifact_id:
            payload["artifact_id"] = self.artifact_id
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationTurn":
        return cls(
            id=str(payload.get("id", "")),
            role=str(payload.get("role", "assistant")),
            content=str(payload.get("content", "")),
            timestamp=str(payload.get("timestamp", "")),
            kind=str(payload.get("kind", "message") or "message"),
            artifact_id=str(payload.get("artifact_id")) if payload.get("artifact_id") is not None else None,
        )


@dataclass
class WorkflowEvent:
    message: str
    timestamp: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "message": self.message,
            "timestamp": self.timestamp,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "WorkflowEvent":
        return cls(
            message=str(payload.get("message", "")),
            timestamp=str(payload.get("timestamp", "")),
        )


@dataclass
class SpecEditProposalChange:
    path: str
    before: str
    after: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "before": self.before,
            "after": self.after,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SpecEditProposalChange":
        return cls(
            path=str(payload.get("path", "")),
            before=str(payload.get("before", "")),
            after=str(payload.get("after", "")),
        )


@dataclass
class SpecEditProposal:
    id: str
    created_at: str
    summary: str
    changes: list[SpecEditProposalChange]
    status: str = "pending"
    canonical_spec_edit_id: Optional[str] = None
    approved_at: Optional[str] = None
    git_branch: Optional[str] = None
    git_commit: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "created_at": self.created_at,
            "summary": self.summary,
            "status": self.status,
            "changes": [change.to_dict() for change in self.changes],
        }
        if self.canonical_spec_edit_id:
            payload["canonical_spec_edit_id"] = self.canonical_spec_edit_id
        if self.approved_at:
            payload["approved_at"] = self.approved_at
        if self.git_branch:
            payload["git_branch"] = self.git_branch
        if self.git_commit:
            payload["git_commit"] = self.git_commit
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SpecEditProposal":
        raw_changes = payload.get("changes")
        changes = [
            SpecEditProposalChange.from_dict(change)
            for change in raw_changes
            if isinstance(change, dict)
        ] if isinstance(raw_changes, list) else []
        return cls(
            id=str(payload.get("id", "")),
            created_at=str(payload.get("created_at", "")),
            summary=str(payload.get("summary", "")),
            changes=changes,
            status=str(payload.get("status", "pending") or "pending"),
            canonical_spec_edit_id=str(payload.get("canonical_spec_edit_id")) if payload.get("canonical_spec_edit_id") is not None else None,
            approved_at=str(payload.get("approved_at")) if payload.get("approved_at") is not None else None,
            git_branch=str(payload.get("git_branch")) if payload.get("git_branch") is not None else None,
            git_commit=str(payload.get("git_commit")) if payload.get("git_commit") is not None else None,
        )


@dataclass
class ExecutionCardReview:
    id: str
    disposition: str
    message: str
    created_at: str
    author: str = "user"

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "disposition": self.disposition,
            "message": self.message,
            "created_at": self.created_at,
            "author": self.author,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCardReview":
        return cls(
            id=str(payload.get("id", "")),
            disposition=str(payload.get("disposition", "")),
            message=str(payload.get("message", "")),
            created_at=str(payload.get("created_at", "")),
            author=str(payload.get("author", "user") or "user"),
        )


@dataclass
class ExecutionCardWorkItem:
    id: str
    title: str
    description: str
    acceptance_criteria: list[str] = field(default_factory=list)
    depends_on: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "acceptance_criteria": list(self.acceptance_criteria),
            "depends_on": list(self.depends_on),
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCardWorkItem":
        raw_acceptance = payload.get("acceptance_criteria")
        raw_depends_on = payload.get("depends_on")
        return cls(
            id=str(payload.get("id", "")),
            title=str(payload.get("title", "")),
            description=str(payload.get("description", "")),
            acceptance_criteria=[str(item) for item in raw_acceptance] if isinstance(raw_acceptance, list) else [],
            depends_on=[str(item) for item in raw_depends_on] if isinstance(raw_depends_on, list) else [],
        )


@dataclass
class ExecutionCard:
    id: str
    title: str
    summary: str
    objective: str
    source_spec_edit_id: str
    source_workflow_run_id: str
    created_at: str
    updated_at: str
    status: str = "draft"
    flow_source: Optional[str] = None
    work_items: list[ExecutionCardWorkItem] = field(default_factory=list)
    review_feedback: list[ExecutionCardReview] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "title": self.title,
            "summary": self.summary,
            "objective": self.objective,
            "source_spec_edit_id": self.source_spec_edit_id,
            "source_workflow_run_id": self.source_workflow_run_id,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "status": self.status,
            "work_items": [item.to_dict() for item in self.work_items],
            "review_feedback": [entry.to_dict() for entry in self.review_feedback],
        }
        if self.flow_source:
            payload["flow_source"] = self.flow_source
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCard":
        raw_items = payload.get("work_items")
        raw_reviews = payload.get("review_feedback")
        return cls(
            id=str(payload.get("id", "")),
            title=str(payload.get("title", "")),
            summary=str(payload.get("summary", "")),
            objective=str(payload.get("objective", "")),
            source_spec_edit_id=str(payload.get("source_spec_edit_id", "")),
            source_workflow_run_id=str(payload.get("source_workflow_run_id", "")),
            created_at=str(payload.get("created_at", "")),
            updated_at=str(payload.get("updated_at", "")),
            status=str(payload.get("status", "draft") or "draft"),
            flow_source=str(payload.get("flow_source")) if payload.get("flow_source") is not None else None,
            work_items=[
                ExecutionCardWorkItem.from_dict(item)
                for item in raw_items
                if isinstance(item, dict)
            ] if isinstance(raw_items, list) else [],
            review_feedback=[
                ExecutionCardReview.from_dict(item)
                for item in raw_reviews
                if isinstance(item, dict)
            ] if isinstance(raw_reviews, list) else [],
        )


@dataclass
class ExecutionWorkflowState:
    run_id: Optional[str] = None
    status: str = "idle"
    error: Optional[str] = None
    flow_source: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "status": self.status,
        }
        if self.run_id:
            payload["run_id"] = self.run_id
        if self.error:
            payload["error"] = self.error
        if self.flow_source:
            payload["flow_source"] = self.flow_source
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionWorkflowState":
        return cls(
            run_id=str(payload.get("run_id")) if payload.get("run_id") is not None else None,
            status=str(payload.get("status", "idle") or "idle"),
            error=str(payload.get("error")) if payload.get("error") is not None else None,
            flow_source=str(payload.get("flow_source")) if payload.get("flow_source") is not None else None,
        )


@dataclass
class ConversationState:
    conversation_id: str
    project_path: str
    turns: list[ConversationTurn] = field(default_factory=list)
    event_log: list[WorkflowEvent] = field(default_factory=list)
    spec_edit_proposals: list[SpecEditProposal] = field(default_factory=list)
    execution_cards: list[ExecutionCard] = field(default_factory=list)
    execution_workflow: ExecutionWorkflowState = field(default_factory=ExecutionWorkflowState)

    def to_dict(self) -> dict[str, Any]:
        return {
            "conversation_id": self.conversation_id,
            "project_path": self.project_path,
            "turns": [turn.to_dict() for turn in self.turns],
            "event_log": [entry.to_dict() for entry in self.event_log],
            "spec_edit_proposals": [proposal.to_dict() for proposal in self.spec_edit_proposals],
            "execution_cards": [card.to_dict() for card in self.execution_cards],
            "execution_workflow": self.execution_workflow.to_dict(),
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationState":
        raw_turns = payload.get("turns")
        raw_events = payload.get("event_log")
        raw_proposals = payload.get("spec_edit_proposals")
        raw_cards = payload.get("execution_cards")
        return cls(
            conversation_id=str(payload.get("conversation_id", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            turns=[
                ConversationTurn.from_dict(turn)
                for turn in raw_turns
                if isinstance(turn, dict)
            ] if isinstance(raw_turns, list) else [],
            event_log=[
                WorkflowEvent.from_dict(entry)
                for entry in raw_events
                if isinstance(entry, dict)
            ] if isinstance(raw_events, list) else [],
            spec_edit_proposals=[
                SpecEditProposal.from_dict(entry)
                for entry in raw_proposals
                if isinstance(entry, dict)
            ] if isinstance(raw_proposals, list) else [],
            execution_cards=[
                ExecutionCard.from_dict(entry)
                for entry in raw_cards
                if isinstance(entry, dict)
            ] if isinstance(raw_cards, list) else [],
            execution_workflow=ExecutionWorkflowState.from_dict(payload.get("execution_workflow", {}))
            if isinstance(payload.get("execution_workflow"), dict)
            else ExecutionWorkflowState(),
        )


class ConversationEventHub:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._subscribers: dict[str, list[asyncio.Queue[dict[str, Any]]]] = {}

    def subscribe(self, conversation_id: str) -> asyncio.Queue[dict[str, Any]]:
        queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue(maxsize=32)
        with self._lock:
            self._subscribers.setdefault(conversation_id, []).append(queue)
        return queue

    def unsubscribe(self, conversation_id: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
        with self._lock:
            listeners = self._subscribers.get(conversation_id)
            if not listeners:
                return
            if queue in listeners:
                listeners.remove(queue)
            if not listeners:
                self._subscribers.pop(conversation_id, None)

    async def publish(self, conversation_id: str, payload: dict[str, Any]) -> None:
        with self._lock:
            listeners = list(self._subscribers.get(conversation_id, []))
        for queue in listeners:
            try:
                queue.put_nowait(payload)
            except asyncio.QueueFull:
                try:
                    queue.get_nowait()
                except asyncio.QueueEmpty:
                    pass
                try:
                    queue.put_nowait(payload)
                except asyncio.QueueFull:
                    continue


class CodexAppServerChatSession:
    def __init__(self, working_dir: str) -> None:
        self.requested_working_dir = _normalize_project_path(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self._proc: Optional[subprocess.Popen[str]] = None
        self._selector: Optional[selectors.DefaultSelector] = None
        self._request_id = 0
        self._thread_id: Optional[str] = None
        self._lock = threading.Lock()

    def _close(self) -> None:
        if self._selector is not None:
            try:
                self._selector.close()
            except Exception:
                pass
            self._selector = None
        if self._proc is not None:
            try:
                if self._proc.poll() is None:
                    self._proc.terminate()
                    try:
                        self._proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        self._proc.kill()
            except Exception:
                pass
            self._proc = None
        self._thread_id = None

    def _ensure_process(self) -> None:
        if self._proc is not None and self._proc.poll() is None:
            return
        self._close()
        try:
            proc = subprocess.Popen(
                ["codex", "app-server"],
                cwd=self.working_dir,
                env=build_codex_runtime_environment(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                start_new_session=True,
            )
        except FileNotFoundError as exc:
            if not Path(self.working_dir).exists():
                raise RuntimeError(
                    "codex app-server working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir or self.working_dir}, resolved {self.working_dir}"
                ) from exc
            raise RuntimeError("codex app-server not found on PATH") from exc
        selector = selectors.DefaultSelector()
        if proc.stdout is None:
            self._close()
            raise RuntimeError("codex app-server did not expose stdout")
        selector.register(proc.stdout, selectors.EVENT_READ)
        self._proc = proc
        self._selector = selector
        self._request_id = 0
        self._thread_id = None
        init_response = self._send_request("initialize", {"clientInfo": {"name": "sparkspawn", "version": "0.1"}})
        if init_response.get("error"):
            self._close()
            raise RuntimeError("codex app-server initialize failed")

    def _send_json(self, payload: dict[str, Any]) -> None:
        if self._proc is None or self._proc.stdin is None:
            raise RuntimeError("codex app-server stdin unavailable")
        self._proc.stdin.write(json.dumps(payload) + "\n")
        self._proc.stdin.flush()

    def _send_response(self, request_id: Any, result: Optional[dict[str, Any]] = None) -> None:
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result or {},
        })

    def _read_line(self, wait: float) -> Optional[str]:
        if self._proc is None or self._selector is None or self._proc.stdout is None:
            return None
        events = self._selector.select(timeout=max(wait, 0))
        if not events:
            return None
        line = self._proc.stdout.readline()
        if not line:
            return None
        return line.rstrip("\n")

    def _handle_server_request(self, message: dict[str, Any]) -> None:
        method = message.get("method")
        request_id = message.get("id")
        if method in {"item/commandExecution/requestApproval", "item/fileChange/requestApproval"}:
            self._send_response(request_id, {"decision": "acceptForSession"})
            return
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": f"Unsupported request: {method}"},
        })

    def _wait_for_response(self, target_id: int) -> dict[str, Any]:
        while True:
            line = self._read_line(0.1)
            if line is None:
                if self._proc is not None and self._proc.poll() is not None:
                    raise RuntimeError("codex app-server exited unexpectedly")
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in message and "method" in message:
                self._handle_server_request(message)
                continue
            if message.get("id") == target_id:
                return message

    def _send_request(self, method: str, params: Optional[dict[str, Any]]) -> dict[str, Any]:
        self._request_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": self._request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self._send_json(payload)
        return self._wait_for_response(self._request_id)

    def _ensure_thread(self, model: Optional[str]) -> None:
        if self._thread_id:
            return
        params: dict[str, Any] = {
            "cwd": self.working_dir,
            "sandbox": "danger-full-access",
            "ephemeral": True,
        }
        if model:
            params["model"] = model
        response = self._send_request("thread/start", params)
        if response.get("error"):
            raise RuntimeError("codex app-server thread/start failed")
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = thread.get("id")
        if not thread_id:
            raise RuntimeError("codex app-server did not return a thread id")
        self._thread_id = str(thread_id)

    def turn(self, prompt: str, model: Optional[str]) -> str:
        with self._lock:
            self._ensure_process()
            self._ensure_thread(model)
            agent_chunks: list[str] = []
            last_error: Optional[str] = None
            params: dict[str, Any] = {
                "threadId": self._thread_id,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": self.working_dir,
            }
            if model:
                params["model"] = model
            response = self._send_request("turn/start", params)
            if response.get("error"):
                raise RuntimeError("codex app-server turn/start failed")
            while True:
                line = self._read_line(0.1)
                if line is None:
                    if self._proc is not None and self._proc.poll() is not None:
                        self._close()
                        raise RuntimeError("codex app-server exited before turn completion")
                    continue
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "id" in message and "method" in message:
                    self._handle_server_request(message)
                    continue
                method = message.get("method")
                params = message.get("params") or {}
                if method == "item/agentMessage/delta":
                    delta = params.get("delta") or ""
                    if delta:
                        agent_chunks.append(str(delta))
                    continue
                if method == "codex/event/agent_message_delta":
                    delta = (params.get("msg") or {}).get("delta") or ""
                    if delta:
                        agent_chunks.append(str(delta))
                    continue
                if method == "codex/event/agent_message":
                    msg = (params.get("msg") or {}).get("message")
                    if msg:
                        agent_chunks = [str(msg)]
                    continue
                if method == "error":
                    last_error = str(params.get("message") or "codex app-server error")
                    continue
                if method == "turn/completed":
                    turn = params.get("turn") or {}
                    status = str(turn.get("status") or "")
                    if status and status != "completed":
                        error = turn.get("error") or {}
                        last_error = str(error.get("message") or last_error or f"turn ended with status '{status}'")
                    break
            if last_error:
                raise RuntimeError(last_error)
            response_text = "".join(agent_chunks).strip()
            if not response_text:
                raise RuntimeError("codex app-server returned an empty chat response")
            return response_text


class ProjectChatService:
    def __init__(self, data_dir: Path) -> None:
        self._data_dir = data_dir
        self._lock = threading.Lock()
        self._event_hub = ConversationEventHub()
        self._sessions_lock = threading.Lock()
        self._sessions: dict[str, CodexAppServerChatSession] = {}

    def events(self) -> ConversationEventHub:
        return self._event_hub

    def _conversations_root(self) -> Path:
        root = self._data_dir / "conversations"
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _conversation_root(self, conversation_id: str) -> Path:
        root = self._conversations_root() / conversation_id
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _conversation_state_path(self, conversation_id: str) -> Path:
        return self._conversation_root(conversation_id) / "state.json"

    def _read_state(self, conversation_id: str) -> Optional[ConversationState]:
        path = self._conversation_state_path(conversation_id)
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None
        state = ConversationState.from_dict(payload)
        if not state.conversation_id:
            state.conversation_id = conversation_id
        return state

    def _write_state(self, state: ConversationState) -> None:
        path = self._conversation_state_path(state.conversation_id)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")

    async def publish_snapshot(self, conversation_id: str) -> None:
        snapshot = self.get_snapshot(conversation_id)
        await self._event_hub.publish(conversation_id, {"type": "conversation_snapshot", "state": snapshot})

    def get_snapshot(self, conversation_id: str, project_path: Optional[str] = None) -> dict[str, Any]:
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                normalized_project_path = _normalize_project_path(project_path or "")
                if not normalized_project_path:
                    raise FileNotFoundError(conversation_id)
                state = ConversationState(
                    conversation_id=conversation_id,
                    project_path=normalized_project_path,
                )
                self._write_state(state)
            elif project_path:
                normalized_project_path = _normalize_project_path(project_path)
                if normalized_project_path and normalized_project_path != state.project_path:
                    raise ValueError("Conversation is already bound to a different project path.")
        return state.to_dict()

    def _append_event(self, state: ConversationState, message: str) -> None:
        state.event_log.append(WorkflowEvent(message=message, timestamp=_iso_now()))

    def _build_chat_prompt(self, state: ConversationState, message: str) -> str:
        recent_turns = state.turns[-10:]
        history_lines = []
        for turn in recent_turns:
            if turn.kind != "message":
                continue
            history_lines.append(f"{turn.role.upper()}: {turn.content}")
        history_text = "\n".join(history_lines) if history_lines else "No prior conversation history."
        return (
            "You are the Sparkspawn project chat assistant. "
            "Respond with JSON only. "
            "Schema: "
            "{\"assistant_message\": string, \"spec_proposal\": null | {\"summary\": string, \"changes\": [{\"path\": string, \"before\": string, \"after\": string}]}}. "
            "Do not create an execution card. "
            "Create a spec_proposal when the user is asking for product, UX, workflow, or implementation-intent changes that should be reviewed before execution. "
            "Keep assistant_message concise and practical.\n\n"
            f"Project path: {state.project_path}\n"
            f"Recent conversation:\n{history_text}\n\n"
            f"Latest user message:\n{message}\n"
        )

    def _build_execution_planning_prompt(
        self,
        state: ConversationState,
        proposal: SpecEditProposal,
        review_feedback: Optional[str],
    ) -> str:
        recent_turns = state.turns[-12:]
        history_lines = []
        for turn in recent_turns:
            if turn.kind != "message":
                continue
            history_lines.append(f"{turn.role.upper()}: {turn.content}")
        history_text = "\n".join(history_lines) if history_lines else "No prior conversation history."
        review_text = review_feedback or "None."
        proposal_payload = json.dumps(proposal.to_dict(), indent=2, sort_keys=True)
        return (
            "You are generating a tracker-ready execution card from an approved spec edit. "
            "Respond with JSON only. "
            "Schema: "
            "{\"title\": string, \"summary\": string, \"objective\": string, "
            "\"work_items\": [{\"id\": string, \"title\": string, \"description\": string, "
            "\"acceptance_criteria\": [string], \"depends_on\": [string]}]}. "
            "Return 1-6 concrete work items. "
            "Do not include markdown fences.\n\n"
            f"Project path: {state.project_path}\n"
            f"Approved spec edit proposal:\n{proposal_payload}\n\n"
            f"Recent conversation:\n{history_text}\n\n"
            f"Latest reviewer feedback for this execution card:\n{review_text}\n"
        )

    def _build_session(self, conversation_id: str, project_path: str) -> CodexAppServerChatSession:
        with self._sessions_lock:
            session = self._sessions.get(conversation_id)
            if session is not None:
                return session
            session = CodexAppServerChatSession(project_path)
            self._sessions[conversation_id] = session
            return session

    def send_turn(self, conversation_id: str, project_path: str, message: str, model: Optional[str]) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Message is required.")
        session = self._build_session(conversation_id, normalized_project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                state = ConversationState(conversation_id=conversation_id, project_path=normalized_project_path)
            elif state.project_path != normalized_project_path:
                raise ValueError("Conversation is already bound to a different project path.")
            user_turn = ConversationTurn(
                id=f"turn-{uuid.uuid4().hex}",
                role="user",
                content=trimmed_message,
                timestamp=_iso_now(),
            )
            state.turns.append(user_turn)
            self._write_state(state)
        raw_response = session.turn(self._build_chat_prompt(state, trimmed_message), model.strip() if model else None)
        parsed = _extract_json_object(raw_response)
        assistant_message = str(parsed.get("assistant_message", "")).strip()
        if not assistant_message:
            assistant_message = "I reviewed that request."
        spec_proposal_payload = parsed.get("spec_proposal")
        with self._lock:
            state = self._read_state(conversation_id) or state
            assistant_turn = ConversationTurn(
                id=f"turn-{uuid.uuid4().hex}",
                role="assistant",
                content=assistant_message,
                timestamp=_iso_now(),
            )
            state.turns.append(assistant_turn)
            if isinstance(spec_proposal_payload, dict):
                raw_changes = spec_proposal_payload.get("changes")
                changes = [
                    SpecEditProposalChange.from_dict(change)
                    for change in raw_changes
                    if isinstance(change, dict)
                ] if isinstance(raw_changes, list) else []
                if changes:
                    proposal = SpecEditProposal(
                        id=f"proposal-{uuid.uuid4().hex[:12]}",
                        created_at=_iso_now(),
                        summary=str(spec_proposal_payload.get("summary", "")).strip() or assistant_message,
                        changes=changes,
                        status="pending",
                    )
                    state.spec_edit_proposals.append(proposal)
                    state.turns.append(
                        ConversationTurn(
                            id=f"turn-{uuid.uuid4().hex}",
                            role="system",
                            content="",
                            timestamp=proposal.created_at,
                            kind="spec_edit_proposal",
                            artifact_id=proposal.id,
                        )
                    )
                    self._append_event(state, f"Drafted spec edit proposal {proposal.id}.")
            self._write_state(state)
        return state.to_dict()

    def reject_spec_edit(self, conversation_id: str, project_path: str, proposal_id: str) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None:
                raise ValueError("Unknown spec edit proposal.")
            proposal.status = "rejected"
            self._append_event(state, f"Rejected spec edit proposal {proposal.id}.")
            self._write_state(state)
        return state.to_dict()

    def _write_spec_edit_artifact(
        self,
        runtime_project_path: str,
        conversation_id: str,
        proposal: SpecEditProposal,
        canonical_spec_edit_id: str,
    ) -> Path:
        artifact_dir = Path(runtime_project_path) / ".attractor" / "spec-edits"
        artifact_dir.mkdir(parents=True, exist_ok=True)
        artifact_path = artifact_dir / f"{canonical_spec_edit_id}.json"
        payload = {
            "canonical_spec_edit_id": canonical_spec_edit_id,
            "conversation_id": conversation_id,
            "approved_at": proposal.approved_at,
            "summary": proposal.summary,
            "changes": [change.to_dict() for change in proposal.changes],
            "proposal_id": proposal.id,
        }
        artifact_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        return artifact_path

    def _git_commit_spec_edit(self, runtime_project_path: str, artifact_path: Path, canonical_spec_edit_id: str) -> tuple[str | None, str | None]:
        project_dir = Path(runtime_project_path)
        relative_artifact_path = artifact_path.relative_to(project_dir)
        subprocess.run(
            ["git", "-C", str(project_dir), "add", str(relative_artifact_path)],
            capture_output=True,
            text=True,
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(project_dir), "commit", "-m", f"Approve spec edit {canonical_spec_edit_id}", "--", str(relative_artifact_path)],
            capture_output=True,
            text=True,
            check=True,
        )
        branch = subprocess.run(
            ["git", "-C", str(project_dir), "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        commit = subprocess.run(
            ["git", "-C", str(project_dir), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        return branch or None, commit or None

    def approve_spec_edit(
        self,
        conversation_id: str,
        project_path: str,
        proposal_id: str,
    ) -> tuple[dict[str, Any], SpecEditProposal]:
        normalized_project_path = _normalize_project_path(project_path)
        runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None:
                raise ValueError("Unknown spec edit proposal.")
            canonical_spec_edit_id = proposal.canonical_spec_edit_id or f"spec-edit-{_slugify(Path(project_path).name)}-{uuid.uuid4().hex[:8]}"
            proposal.status = "applied"
            proposal.canonical_spec_edit_id = canonical_spec_edit_id
            proposal.approved_at = _iso_now()
        if not Path(runtime_project_path).exists():
            raise ValueError(
                "Project path is not available inside the backend runtime: "
                f"requested {normalized_project_path}, resolved {runtime_project_path}"
            )
        artifact_path = self._write_spec_edit_artifact(runtime_project_path, conversation_id, proposal, canonical_spec_edit_id)
        branch, commit = self._git_commit_spec_edit(runtime_project_path, artifact_path, canonical_spec_edit_id)
        with self._lock:
            state = self._read_state(conversation_id) or state
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), proposal)
            proposal.status = "applied"
            proposal.canonical_spec_edit_id = canonical_spec_edit_id
            proposal.approved_at = proposal.approved_at or _iso_now()
            proposal.git_branch = branch
            proposal.git_commit = commit
            self._append_event(
                state,
                f"Approved spec edit proposal {proposal.id} as canonical spec edit {canonical_spec_edit_id} and committed it to git.",
            )
            self._write_state(state)
        return state.to_dict(), proposal

    def mark_execution_workflow_started(
        self,
        conversation_id: str,
        workflow_run_id: str,
        flow_source: Optional[str],
    ) -> dict[str, Any]:
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                raise ValueError("Unknown conversation.")
            state.execution_workflow = ExecutionWorkflowState(
                run_id=workflow_run_id,
                status="running",
                error=None,
                flow_source=flow_source,
            )
            if flow_source:
                self._append_event(state, f"Execution planning started ({workflow_run_id}) using {flow_source}.")
            else:
                self._append_event(state, f"Execution planning started ({workflow_run_id}).")
            self._write_state(state)
        return state.to_dict()

    async def run_execution_workflow(
        self,
        conversation_id: str,
        proposal_id: str,
        model: Optional[str],
        flow_source: Optional[str],
        review_feedback: Optional[str],
        workflow_run_id: str,
        codex_runner: Any,
    ) -> None:
        try:
            state = self._read_state(conversation_id)
            if state is None:
                return
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None or not proposal.canonical_spec_edit_id:
                raise RuntimeError("Approved spec edit proposal was not found.")
            prompt = self._build_execution_planning_prompt(state, proposal, review_feedback)
            raw_response = await asyncio.to_thread(codex_runner, prompt, model)
            parsed = _extract_json_object(raw_response)
            raw_items = parsed.get("work_items")
            work_items = [
                ExecutionCardWorkItem.from_dict(item)
                for item in raw_items
                if isinstance(item, dict)
            ] if isinstance(raw_items, list) else []
            if not work_items:
                raise RuntimeError("Execution planning returned no work items.")
            now = _iso_now()
            execution_card = ExecutionCard(
                id=f"execution-card-{uuid.uuid4().hex[:12]}",
                title=str(parsed.get("title", "")).strip() or "Execution plan",
                summary=str(parsed.get("summary", "")).strip() or "Generated execution plan.",
                objective=str(parsed.get("objective", "")).strip() or "Implement the approved spec edit.",
                source_spec_edit_id=proposal.canonical_spec_edit_id,
                source_workflow_run_id=workflow_run_id,
                created_at=now,
                updated_at=now,
                status="draft",
                flow_source=flow_source,
                work_items=work_items,
            )
            with self._lock:
                state = self._read_state(conversation_id)
                if state is None:
                    return
                state.execution_cards.append(execution_card)
                state.turns.append(
                    ConversationTurn(
                        id=f"turn-{uuid.uuid4().hex}",
                        role="system",
                        content="",
                        timestamp=now,
                        kind="execution_card",
                        artifact_id=execution_card.id,
                    )
                )
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="idle",
                    error=None,
                    flow_source=flow_source,
                )
                self._append_event(state, f"Execution planning completed and produced {execution_card.id}.")
                self._write_state(state)
            await self.publish_snapshot(conversation_id)
        except Exception as exc:  # noqa: BLE001
            with self._lock:
                state = self._read_state(conversation_id)
                if state is None:
                    return
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="failed",
                    error=str(exc),
                    flow_source=flow_source,
                )
                self._append_event(state, f"Execution planning failed: {exc}")
                self._write_state(state)
            await self.publish_snapshot(conversation_id)

    def review_execution_card(
        self,
        conversation_id: str,
        project_path: str,
        execution_card_id: str,
        disposition: str,
        message: str,
        flow_source: Optional[str],
    ) -> tuple[dict[str, Any], ExecutionCard, Optional[str], Optional[str]]:
        normalized_project_path = _normalize_project_path(project_path)
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Review message is required.")
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            execution_card = next((entry for entry in state.execution_cards if entry.id == execution_card_id), None)
            if execution_card is None:
                raise ValueError("Unknown execution card.")
            effective_flow_source = flow_source or execution_card.flow_source
            now = _iso_now()
            state.turns.append(
                ConversationTurn(
                    id=f"turn-{uuid.uuid4().hex}",
                    role="user",
                    content=trimmed_message,
                    timestamp=now,
                )
            )
            execution_card.review_feedback.append(
                ExecutionCardReview(
                    id=f"review-{uuid.uuid4().hex[:12]}",
                    disposition=disposition,
                    message=trimmed_message,
                    created_at=now,
                )
            )
            if disposition == "approved":
                execution_card.status = "approved"
                execution_card.updated_at = now
                self._append_event(state, f"Approved execution card {execution_card.id}.")
                workflow_run_id = None
                source_proposal_id = None
            else:
                execution_card.status = "revision-requested" if disposition == "revision_requested" else "rejected"
                execution_card.updated_at = now
                workflow_run_id = f"workflow-{uuid.uuid4().hex[:12]}"
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="running",
                    error=None,
                    flow_source=effective_flow_source,
                )
                source_proposal_id = next(
                    (
                        proposal.id
                        for proposal in reversed(state.spec_edit_proposals)
                        if proposal.canonical_spec_edit_id == execution_card.source_spec_edit_id
                    ),
                    None,
                )
                self._append_event(
                    state,
                    f"{'Requested revision for' if disposition == 'revision_requested' else 'Rejected'} execution card {execution_card.id}; regenerating with reviewer feedback.",
                )
            self._write_state(state)
        return state.to_dict(), execution_card, source_proposal_id, workflow_run_id
