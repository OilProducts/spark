from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

_WRITABLE_DIRECTORY_PROBE = ".writable-directory-probe"


@dataclass(frozen=True)
class AttractorRuntimePaths:
    runtime_dir: Path
    runs_dir: Path
    flows_dir: Path
    config_dir: Path


def resolve_runtime_paths(
    *,
    runtime_dir: Path | str | None,
    runs_dir: Path | str | None,
    flows_dir: Path | str | None,
    config_dir: Path | str | None = None,
) -> AttractorRuntimePaths:
    if runtime_dir is None or runs_dir is None or flows_dir is None:
        raise RuntimeError(
            "Attractor runtime paths must include runtime_dir, runs_dir, and flows_dir."
        )
    return AttractorRuntimePaths(
        runtime_dir=_normalize_path(runtime_dir),
        runs_dir=_normalize_path(runs_dir),
        flows_dir=_normalize_path(flows_dir),
        config_dir=_normalize_path(config_dir or (Path(runtime_dir).expanduser().resolve(strict=False).parent / "config")),
    )


def validate_runtime_paths(paths: AttractorRuntimePaths) -> None:
    ensure_writable_directory(paths.runtime_dir, "runtime")
    ensure_writable_directory(paths.runs_dir, "runs")
    ensure_writable_directory(paths.flows_dir, "flows")
    ensure_writable_directory(paths.config_dir, "config")


def ensure_writable_directory(path: Path, label: str) -> None:
    target = path.resolve(strict=False)
    try:
        target.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise RuntimeError(f"Unable to create {label} directory: {target}") from exc

    probe = target / _WRITABLE_DIRECTORY_PROBE
    try:
        probe.write_text("ok", encoding="utf-8")
        probe.unlink()
    except OSError as exc:
        raise RuntimeError(f"{label} directory is not writable: {target}") from exc


def _normalize_path(value: Path | str) -> Path:
    return Path(value).expanduser().resolve(strict=False)
