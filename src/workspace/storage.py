from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import secrets
import shutil
import tomllib
from typing import Any

from sparkspawn_common.logging import get_sparkspawn_logger
from sparkspawn_common.runtime import build_project_id, normalize_project_path


LOGGER = get_sparkspawn_logger("workspace.storage")


def _iso_now() -> str:
    from time import gmtime, strftime

    return strftime("%Y-%m-%dT%H:%M:%SZ", gmtime())


_UNSET = object()
_CONVERSATION_HANDLE_SCHEMA_VERSION = 1
_CONVERSATION_HANDLE_PATTERN = "adjective-noun"

_HANDLE_ADJECTIVES = (
    "amber", "ancient", "autumn", "bold", "brisk", "calm", "cedar", "clear", "cloudy", "cobalt",
    "crisp", "curious", "daily", "daring", "deep", "delicate", "eager", "early", "electric", "ember",
    "faint", "fancy", "fast", "fern", "fierce", "final", "forest", "fresh", "gentle", "glossy",
    "golden", "grand", "graphic", "green", "hidden", "hollow", "honest", "icy", "jagged", "juniper",
    "keen", "kind", "lattice", "light", "lively", "lunar", "mellow", "midnight", "misty", "modern",
    "mossy", "navy", "nimble", "noble", "north", "odd", "olive", "open", "orange", "patient",
    "pearl", "pine", "plain", "polished", "prairie", "proud", "quick", "quiet", "rapid", "rare",
    "red", "remote", "river", "robust", "rocky", "royal", "rustic", "sage", "scarlet", "shadow",
    "sharp", "silver", "simple", "sky", "small", "smoky", "solar", "solid", "spring", "steady",
    "stone", "stormy", "summer", "sunny", "swift", "tidy", "timber", "tiny", "topaz", "tranquil",
    "true", "urban", "vivid", "warm", "western", "white", "wild", "winter", "wise", "wooden",
)

_HANDLE_NOUNS = (
    "anchor", "antler", "arch", "arrow", "ash", "badger", "bank", "barley", "bay", "beacon",
    "berry", "bird", "blossom", "bridge", "brook", "brush", "cabin", "canyon", "cardinal", "cedar",
    "circle", "cliff", "cloud", "coast", "comet", "creek", "crest", "crow", "delta", "dove",
    "drift", "dune", "echo", "falcon", "field", "finch", "firefly", "fjord", "flower", "forest",
    "forge", "fox", "garden", "glade", "grain", "grove", "harbor", "hawk", "hazel", "hill",
    "hollow", "island", "jet", "juniper", "kingfisher", "lake", "lantern", "leaf", "line", "lily",
    "meadow", "mesa", "moon", "mountain", "otter", "owl", "peak", "pebble", "pine", "planet",
    "pond", "prairie", "quartz", "raven", "reef", "ridge", "river", "robin", "sail", "sandpiper",
    "shadow", "shore", "signal", "sky", "snowflake", "sparrow", "spring", "spruce", "star", "stone",
    "stream", "summit", "sunrise", "swallow", "thicket", "thistle", "timber", "trail", "valley", "wave",
    "willow", "wind", "wren", "yard", "zephyr",
)


@dataclass(frozen=True)
class ProjectPaths:
    project_id: str
    project_path: str
    display_name: str
    root: Path
    project_file: Path
    conversations_dir: Path
    workflow_dir: Path
    proposals_dir: Path
    flow_run_requests_dir: Path
    execution_cards_dir: Path


@dataclass(frozen=True)
class ProjectRecord:
    project_id: str
    project_path: str
    display_name: str
    created_at: str
    last_opened_at: str
    last_accessed_at: str | None
    is_favorite: bool
    active_conversation_id: str | None
    flow_bindings: dict[str, str]


@dataclass(frozen=True)
class DeletedProjectRecord:
    project_id: str
    project_path: str
    display_name: str


def ensure_project_paths(home_dir: Path, project_path: str) -> ProjectPaths:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        raise ValueError("Project path is required.")

    project_id = build_project_id(normalized_project_path)
    display_name = Path(normalized_project_path).name or normalized_project_path
    project_root = workspace_projects_root(home_dir) / project_id
    project_file = project_root / "project.toml"
    conversations_dir = project_root / "conversations"
    workflow_dir = project_root / "workflow"
    proposals_dir = project_root / "proposals"
    flow_run_requests_dir = project_root / "flow-run-requests"
    execution_cards_dir = project_root / "execution-cards"

    for directory in (
        project_root,
        conversations_dir,
        workflow_dir,
        proposals_dir,
        flow_run_requests_dir,
        execution_cards_dir,
    ):
        directory.mkdir(parents=True, exist_ok=True)

    payload = _read_project_record(project_file)
    created_at = str(payload.get("created_at", "") or "")
    if not created_at:
        created_at = _iso_now()

    _write_project_record(
        project_file,
        {
            "project_id": project_id,
            "project_path": normalized_project_path,
            "display_name": display_name,
            "created_at": created_at,
            "last_opened_at": _iso_now(),
            "last_accessed_at": _read_optional_string(payload, "last_accessed_at"),
            "is_favorite": _read_optional_bool(payload, "is_favorite", default=False),
            "active_conversation_id": _read_optional_string(payload, "active_conversation_id"),
            "flow_bindings": _read_flow_bindings(payload),
        },
    )

    return ProjectPaths(
        project_id=project_id,
        project_path=normalized_project_path,
        display_name=display_name,
        root=project_root,
        project_file=project_file,
        conversations_dir=conversations_dir,
        workflow_dir=workflow_dir,
        proposals_dir=proposals_dir,
        flow_run_requests_dir=flow_run_requests_dir,
        execution_cards_dir=execution_cards_dir,
    )


def read_project_paths_by_id(home_dir: Path, project_id: str) -> ProjectPaths | None:
    root = workspace_projects_root(home_dir) / project_id
    project_file = root / "project.toml"
    if not project_file.exists():
        return None
    payload = _read_project_record(project_file)
    project_path = normalize_project_path(str(payload.get("project_path", "")))
    if not project_path:
        LOGGER.warning("Skipping project record with missing or invalid project_path in %s", project_file)
        return None
    display_name = str(payload.get("display_name", "") or Path(project_path).name or project_path)
    return ProjectPaths(
        project_id=project_id,
        project_path=project_path,
        display_name=display_name,
        root=root,
        project_file=project_file,
        conversations_dir=root / "conversations",
        workflow_dir=root / "workflow",
        proposals_dir=root / "proposals",
        flow_run_requests_dir=root / "flow-run-requests",
        execution_cards_dir=root / "execution-cards",
    )


def read_project_record(home_dir: Path, project_path: str) -> ProjectRecord | None:
    project_paths = ensure_project_paths(home_dir, project_path)
    return read_project_record_by_id(home_dir, project_paths.project_id)


def read_project_record_by_id(home_dir: Path, project_id: str) -> ProjectRecord | None:
    project_paths = read_project_paths_by_id(home_dir, project_id)
    if project_paths is None:
        return None
    return _build_project_record(project_paths)


def list_project_records(home_dir: Path) -> list[ProjectRecord]:
    projects_root = workspace_projects_root(home_dir)
    if not projects_root.exists():
        return []
    records: list[ProjectRecord] = []
    for project_root in sorted(projects_root.iterdir()):
        if not project_root.is_dir():
            continue
        record = read_project_record_by_id(home_dir, project_root.name)
        if record is not None:
            records.append(record)
    return records


def update_project_record(
    home_dir: Path,
    project_path: str,
    *,
    display_name: str | None = None,
    last_accessed_at: str | None | object = _UNSET,
    is_favorite: bool | object = _UNSET,
    active_conversation_id: str | None | object = _UNSET,
) -> ProjectRecord:
    project_paths = ensure_project_paths(home_dir, project_path)
    payload = _read_project_record(project_paths.project_file)
    next_payload: dict[str, Any] = {
        "project_id": project_paths.project_id,
        "project_path": project_paths.project_path,
        "display_name": display_name or str(payload.get("display_name", "") or project_paths.display_name),
        "created_at": str(payload.get("created_at", "") or _iso_now()),
        "last_opened_at": str(payload.get("last_opened_at", "") or _iso_now()),
        "last_accessed_at": _read_optional_string(payload, "last_accessed_at"),
        "is_favorite": _read_optional_bool(payload, "is_favorite", default=False),
        "active_conversation_id": _read_optional_string(payload, "active_conversation_id"),
        "flow_bindings": _read_flow_bindings(payload),
    }
    if last_accessed_at is not _UNSET:
        next_payload["last_accessed_at"] = _normalize_optional_string(last_accessed_at)
    if is_favorite is not _UNSET:
        next_payload["is_favorite"] = bool(is_favorite)
    if active_conversation_id is not _UNSET:
        next_payload["active_conversation_id"] = _normalize_optional_string(active_conversation_id)
    _write_project_record(project_paths.project_file, next_payload)
    return _build_project_record(project_paths)


def workspace_projects_root(home_dir: Path) -> Path:
    root = home_dir / "workspace" / "projects"
    root.mkdir(parents=True, exist_ok=True)
    return root


def workspace_root(home_dir: Path) -> Path:
    root = home_dir / "workspace"
    root.mkdir(parents=True, exist_ok=True)
    return root


def conversation_handles_path(home_dir: Path) -> Path:
    return workspace_root(home_dir) / "conversation-handles.json"


def load_conversation_handle_index(home_dir: Path) -> dict[str, Any]:
    path = conversation_handles_path(home_dir)
    if not path.exists():
        return {
            "schema_version": _CONVERSATION_HANDLE_SCHEMA_VERSION,
            "pattern": _CONVERSATION_HANDLE_PATTERN,
            "handles": {},
            "conversation_ids": {},
        }
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        payload = {}
    if not isinstance(payload, dict):
        payload = {}
    handles = payload.get("handles")
    conversation_ids = payload.get("conversation_ids")
    return {
        "schema_version": _CONVERSATION_HANDLE_SCHEMA_VERSION,
        "pattern": _CONVERSATION_HANDLE_PATTERN,
        "handles": handles if isinstance(handles, dict) else {},
        "conversation_ids": conversation_ids if isinstance(conversation_ids, dict) else {},
    }


def write_conversation_handle_index(home_dir: Path, payload: dict[str, Any]) -> None:
    path = conversation_handles_path(home_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")


def ensure_conversation_handle(
    home_dir: Path,
    *,
    conversation_id: str,
    project_id: str,
    project_path: str,
    created_at: str,
    preferred_handle: str | None = None,
) -> str:
    payload = load_conversation_handle_index(home_dir)
    conversation_ids = payload["conversation_ids"]
    handles = payload["handles"]
    existing_handle = conversation_ids.get(conversation_id)
    if isinstance(existing_handle, str):
        existing_entry = handles.get(existing_handle)
        if isinstance(existing_entry, dict):
            return existing_handle
    normalized_preferred_handle = normalize_conversation_handle(preferred_handle or "")
    if normalized_preferred_handle and normalized_preferred_handle not in handles:
        handles[normalized_preferred_handle] = {
            "conversation_id": conversation_id,
            "project_id": project_id,
            "project_path": project_path,
            "created_at": created_at,
        }
        conversation_ids[conversation_id] = normalized_preferred_handle
        write_conversation_handle_index(home_dir, payload)
        return normalized_preferred_handle
    for _ in range(2048):
        candidate = _generate_conversation_handle()
        if candidate in handles:
            continue
        handles[candidate] = {
            "conversation_id": conversation_id,
            "project_id": project_id,
            "project_path": project_path,
            "created_at": created_at,
        }
        conversation_ids[conversation_id] = candidate
        write_conversation_handle_index(home_dir, payload)
        return candidate
    raise RuntimeError("Could not allocate a unique conversation handle.")


def find_conversation_by_handle(home_dir: Path, handle: str) -> dict[str, str] | None:
    normalized_handle = normalize_conversation_handle(handle)
    if not normalized_handle:
        return None
    payload = load_conversation_handle_index(home_dir)
    entry = payload["handles"].get(normalized_handle)
    if not isinstance(entry, dict):
        return None
    conversation_id = entry.get("conversation_id")
    project_id = entry.get("project_id")
    project_path = entry.get("project_path")
    if not isinstance(conversation_id, str) or not isinstance(project_id, str) or not isinstance(project_path, str):
        return None
    return {
        "conversation_id": conversation_id,
        "project_id": project_id,
        "project_path": project_path,
        "conversation_handle": normalized_handle,
    }


def remove_conversation_handle(home_dir: Path, conversation_id: str) -> None:
    payload = load_conversation_handle_index(home_dir)
    conversation_ids = payload["conversation_ids"]
    handles = payload["handles"]
    existing_handle = conversation_ids.pop(conversation_id, None)
    if isinstance(existing_handle, str):
        handles.pop(existing_handle, None)
        write_conversation_handle_index(home_dir, payload)


def remove_project_conversation_handles(home_dir: Path, project_id: str) -> None:
    payload = load_conversation_handle_index(home_dir)
    handles = payload["handles"]
    conversation_ids = payload["conversation_ids"]
    removed = False
    for handle, record in list(handles.items()):
        if not isinstance(record, dict) or record.get("project_id") != project_id:
            continue
        conversation_id = record.get("conversation_id")
        if isinstance(conversation_id, str):
            conversation_ids.pop(conversation_id, None)
        handles.pop(handle, None)
        removed = True
    if removed:
        write_conversation_handle_index(home_dir, payload)


def normalize_conversation_handle(value: str) -> str:
    trimmed = value.strip().lower()
    if not trimmed:
        return ""
    left, separator, right = trimmed.partition("-")
    if separator != "-" or not left.isalpha() or not right.isalpha():
        return ""
    return f"{left}-{right}"


def delete_project_record(home_dir: Path, project_path: str) -> DeletedProjectRecord:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        raise ValueError("Project path is required.")

    project_id = build_project_id(normalized_project_path)
    project_paths = read_project_paths_by_id(home_dir, project_id)
    if project_paths is None or project_paths.project_path != normalized_project_path:
        raise ValueError("Unknown project.")

    deleted = DeletedProjectRecord(
        project_id=project_paths.project_id,
        project_path=project_paths.project_path,
        display_name=project_paths.display_name,
    )
    shutil.rmtree(project_paths.root, ignore_errors=False)
    remove_project_conversation_handles(home_dir, project_paths.project_id)
    return deleted


def set_project_flow_binding(home_dir: Path, project_path: str, trigger: str, flow_name: str) -> ProjectRecord:
    project_paths = ensure_project_paths(home_dir, project_path)
    payload = _read_project_record(project_paths.project_file)
    next_flow_bindings = _read_flow_bindings(payload)
    next_flow_bindings[trigger] = flow_name
    _write_project_record(
        project_paths.project_file,
        {
            "project_id": project_paths.project_id,
            "project_path": project_paths.project_path,
            "display_name": str(payload.get("display_name", "") or project_paths.display_name),
            "created_at": str(payload.get("created_at", "") or _iso_now()),
            "last_opened_at": str(payload.get("last_opened_at", "") or _iso_now()),
            "last_accessed_at": _read_optional_string(payload, "last_accessed_at"),
            "is_favorite": _read_optional_bool(payload, "is_favorite", default=False),
            "active_conversation_id": _read_optional_string(payload, "active_conversation_id"),
            "flow_bindings": next_flow_bindings,
        },
    )
    return _build_project_record(project_paths)


def delete_project_flow_binding(home_dir: Path, project_path: str, trigger: str) -> ProjectRecord:
    project_paths = ensure_project_paths(home_dir, project_path)
    payload = _read_project_record(project_paths.project_file)
    next_flow_bindings = _read_flow_bindings(payload)
    next_flow_bindings.pop(trigger, None)
    _write_project_record(
        project_paths.project_file,
        {
            "project_id": project_paths.project_id,
            "project_path": project_paths.project_path,
            "display_name": str(payload.get("display_name", "") or project_paths.display_name),
            "created_at": str(payload.get("created_at", "") or _iso_now()),
            "last_opened_at": str(payload.get("last_opened_at", "") or _iso_now()),
            "last_accessed_at": _read_optional_string(payload, "last_accessed_at"),
            "is_favorite": _read_optional_bool(payload, "is_favorite", default=False),
            "active_conversation_id": _read_optional_string(payload, "active_conversation_id"),
            "flow_bindings": next_flow_bindings,
        },
    )
    return _build_project_record(project_paths)


def _read_project_record(path: Path) -> dict[str, object]:
    if not path.exists():
        return {}
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        LOGGER.warning("Failed to read project record from %s: %s", path, exc)
        return {}


def _normalize_optional_string(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    trimmed = value.strip()
    return trimmed or None


def _read_optional_string(payload: dict[str, object], key: str) -> str | None:
    return _normalize_optional_string(payload.get(key))


def _read_optional_bool(payload: dict[str, object], key: str, *, default: bool) -> bool:
    value = payload.get(key)
    if isinstance(value, bool):
        return value
    return default


def _read_flow_bindings(payload: dict[str, object]) -> dict[str, str]:
    raw_bindings = payload.get("flow_bindings")
    if not isinstance(raw_bindings, dict):
        return {}
    next_bindings: dict[str, str] = {}
    for raw_trigger, raw_flow_name in raw_bindings.items():
        if not isinstance(raw_trigger, str) or not isinstance(raw_flow_name, str):
            continue
        trigger = raw_trigger.strip()
        flow_name = raw_flow_name.strip()
        if not trigger or not flow_name:
            continue
        next_bindings[trigger] = flow_name
    return next_bindings


def _build_project_record(project_paths: ProjectPaths) -> ProjectRecord:
    payload = _read_project_record(project_paths.project_file)
    return ProjectRecord(
        project_id=project_paths.project_id,
        project_path=project_paths.project_path,
        display_name=str(payload.get("display_name", "") or project_paths.display_name),
        created_at=str(payload.get("created_at", "") or ""),
        last_opened_at=str(payload.get("last_opened_at", "") or ""),
        last_accessed_at=_read_optional_string(payload, "last_accessed_at"),
        is_favorite=_read_optional_bool(payload, "is_favorite", default=False),
        active_conversation_id=_read_optional_string(payload, "active_conversation_id"),
        flow_bindings=_read_flow_bindings(payload),
    )


def _toml_string(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace("\"", "\\\"")
    return f"\"{escaped}\""


def _toml_bool(value: bool) -> str:
    return "true" if value else "false"


def _write_project_record(path: Path, payload: dict[str, Any]) -> None:
    lines = [
        f"project_id = {_toml_string(payload['project_id'])}",
        f"project_path = {_toml_string(payload['project_path'])}",
        f"display_name = {_toml_string(payload['display_name'])}",
        f"created_at = {_toml_string(payload['created_at'])}",
        f"last_opened_at = {_toml_string(payload['last_opened_at'])}",
    ]
    if payload.get("last_accessed_at"):
        lines.append(f"last_accessed_at = {_toml_string(str(payload['last_accessed_at']))}")
    lines.append(f"is_favorite = {_toml_bool(bool(payload.get('is_favorite', False)))}")
    if payload.get("active_conversation_id"):
        lines.append(f"active_conversation_id = {_toml_string(str(payload['active_conversation_id']))}")
    flow_bindings = payload.get("flow_bindings")
    if isinstance(flow_bindings, dict) and flow_bindings:
        lines.append("")
        lines.append("[flow_bindings]")
        for trigger in sorted(flow_bindings.keys()):
            flow_name = flow_bindings.get(trigger)
            if not isinstance(trigger, str) or not isinstance(flow_name, str):
                continue
            lines.append(f"{trigger} = {_toml_string(flow_name)}")
    lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def _generate_conversation_handle() -> str:
    return f"{secrets.choice(_HANDLE_ADJECTIVES)}-{secrets.choice(_HANDLE_NOUNS)}"
