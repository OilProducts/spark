from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
from typing import Mapping

from attractor.api.runtime_paths import ensure_writable_directory

ENV_HOME_DIR = "SPARK_HOME"
ENV_FLOWS_DIR = "SPARK_FLOWS_DIR"
ENV_UI_DIR = "SPARK_UI_DIR"
ENV_PROJECT_ROOTS = "SPARK_PROJECT_ROOTS"


@dataclass(frozen=True)
class SparkSettings:
    project_root: Path
    data_dir: Path
    config_dir: Path
    runtime_dir: Path
    logs_dir: Path
    workspace_dir: Path
    projects_dir: Path
    attractor_dir: Path
    runs_dir: Path
    flows_dir: Path
    ui_dir: Path | None
    project_roots: tuple[Path, ...]


def resolve_settings(
    *,
    data_dir: Path | str | None = None,
    runs_dir: Path | str | None = None,
    flows_dir: Path | str | None = None,
    ui_dir: Path | str | None = None,
    env: Mapping[str, str] | None = None,
) -> SparkSettings:
    env_map = env if env is not None else os.environ
    project_root = _detect_project_root()
    default_data_dir = Path.home() / ".spark"

    resolved_data_dir = _coalesce_path(
        cli_value=data_dir,
        env_value=env_map.get(ENV_HOME_DIR),
        default_value=default_data_dir,
    )
    resolved_config_dir = resolved_data_dir / "config"
    resolved_runtime_dir = resolved_data_dir / "runtime"
    resolved_logs_dir = resolved_data_dir / "logs"
    resolved_workspace_dir = resolved_data_dir / "workspace"
    resolved_projects_dir = resolved_workspace_dir / "projects"
    resolved_attractor_dir = resolved_data_dir / "attractor"
    resolved_runs_dir = _coalesce_path(
        cli_value=runs_dir,
        env_value=None,
        default_value=resolved_attractor_dir / "runs",
    )
    resolved_flows_dir = _coalesce_path(
        cli_value=flows_dir,
        env_value=env_map.get(ENV_FLOWS_DIR),
        default_value=resolved_data_dir / "flows",
    )
    resolved_ui_dir = _coalesce_optional_path(
        cli_value=ui_dir,
        env_value=env_map.get(ENV_UI_DIR),
        default_value=None,
    )
    resolved_project_roots = _parse_project_roots(env_map.get(ENV_PROJECT_ROOTS))

    return SparkSettings(
        project_root=project_root,
        data_dir=resolved_data_dir,
        config_dir=resolved_config_dir,
        runtime_dir=resolved_runtime_dir,
        logs_dir=resolved_logs_dir,
        workspace_dir=resolved_workspace_dir,
        projects_dir=resolved_projects_dir,
        attractor_dir=resolved_attractor_dir,
        runs_dir=resolved_runs_dir,
        flows_dir=resolved_flows_dir,
        ui_dir=resolved_ui_dir,
        project_roots=resolved_project_roots,
    )


def validate_settings(settings: SparkSettings) -> None:
    ensure_writable_directory(settings.config_dir, "config")
    ensure_writable_directory(settings.runtime_dir, "runtime")
    ensure_writable_directory(settings.logs_dir, "logs")
    ensure_writable_directory(settings.workspace_dir, "workspace")
    ensure_writable_directory(settings.projects_dir, "projects")
    ensure_writable_directory(settings.attractor_dir, "attractor")
    ensure_writable_directory(settings.runs_dir, "runs")
    ensure_writable_directory(settings.flows_dir, "flows")
    if settings.ui_dir:
        ui_index = settings.ui_dir / "index.html"
        if not ui_index.exists():
            raise RuntimeError(f"UI directory does not contain index.html: {settings.ui_dir}")


def _detect_project_root() -> Path:
    return Path(__file__).resolve().parents[2]


def _coalesce_path(
    *,
    cli_value: Path | str | None,
    env_value: str | None,
    default_value: Path,
) -> Path:
    if cli_value is not None:
        return _normalize_path(cli_value)
    if env_value:
        return _normalize_path(env_value)
    return _normalize_path(default_value)


def _coalesce_optional_path(
    *,
    cli_value: Path | str | None,
    env_value: str | None,
    default_value: Path | None,
) -> Path | None:
    if cli_value is not None:
        return _normalize_path(cli_value)
    if env_value:
        return _normalize_path(env_value)
    if default_value is None:
        return None
    return _normalize_path(default_value)


def _normalize_path(value: Path | str) -> Path:
    return Path(value).expanduser().resolve(strict=False)


def _parse_project_roots(value: str | None) -> tuple[Path, ...]:
    if not value:
        return ()

    roots: list[Path] = []
    for entry in value.split(os.pathsep):
        trimmed_entry = entry.strip()
        if not trimmed_entry:
            continue
        if not Path(trimmed_entry).expanduser().is_absolute():
            continue
        path = _normalize_path(trimmed_entry)
        roots.append(path)
    return tuple(roots)
