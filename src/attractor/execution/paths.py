from __future__ import annotations

from pathlib import Path

from .errors import ExecutionLaunchError
from .models import ExecutionProfile


def map_remote_project_path(profile: ExecutionProfile, control_project_path: str | Path) -> Path:
    """Map a control-plane project path into a remote worker project path."""
    if not profile.is_remote_worker:
        raise ExecutionLaunchError("remote project path mapping requires a remote_worker profile")
    if profile.control_project_root is None:
        raise ExecutionLaunchError("remote_worker profile is missing control_project_root")
    if profile.worker_project_root is None:
        raise ExecutionLaunchError("remote_worker profile is missing worker_project_root")

    control_root = _normalize_path(profile.control_project_root)
    worker_root = _normalize_path(profile.worker_project_root)
    control_path = _normalize_path(control_project_path)

    try:
        relative_suffix = control_path.relative_to(control_root)
    except ValueError as exc:
        raise ExecutionLaunchError(
            f"control project path {control_path} is outside remote control_project_root {control_root}"
        ) from exc

    return worker_root if str(relative_suffix) == "." else worker_root / relative_suffix


def _normalize_path(value: str | Path) -> Path:
    raw = Path(value).expanduser()
    parts: list[str] = []
    for part in raw.parts:
        if part in {"", "."}:
            continue
        if part == "..":
            if parts and parts[-1] not in {raw.anchor, ".."}:
                parts.pop()
            else:
                parts.append(part)
            continue
        parts.append(part)
    if not parts:
        return Path(".")
    if raw.anchor and (not parts or parts[0] != raw.anchor):
        parts.insert(0, raw.anchor)
    return Path(*parts)
