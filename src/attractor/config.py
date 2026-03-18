from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
from typing import Mapping, Optional

ENV_HOME_DIR = "SPARKSPAWN_HOME"
ENV_FLOWS_DIR = "SPARKSPAWN_FLOWS_DIR"
ENV_UI_DIR = "SPARKSPAWN_UI_DIR"


@dataclass(frozen=True)
class Settings:
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
    ui_dir: Optional[Path]
    legacy_ui_index: Optional[Path]


def resolve_settings(
    *,
    data_dir: Path | str | None = None,
    runs_dir: Path | str | None = None,
    flows_dir: Path | str | None = None,
    ui_dir: Path | str | None = None,
    env: Mapping[str, str] | None = None,
) -> Settings:
    env_map = env if env is not None else os.environ
    project_root = _detect_project_root()
    default_data_dir = Path.home() / ".sparkspawn"
    default_config_dir = default_data_dir / "config"
    default_runtime_dir = default_data_dir / "runtime"
    default_logs_dir = default_data_dir / "logs"
    default_flows_dir = default_data_dir / "flows"

    default_ui_dir = _resolve_default_ui_dir(project_root)
    legacy_ui_index = project_root / "index.html"
    if not legacy_ui_index.exists():
        legacy_ui_index = None

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
        default_value=default_flows_dir,
    )

    resolved_ui_dir = _coalesce_optional_path(
        cli_value=ui_dir,
        env_value=env_map.get(ENV_UI_DIR),
        default_value=default_ui_dir,
    )

    return Settings(
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
        legacy_ui_index=legacy_ui_index,
    )


def validate_settings(settings: Settings) -> None:
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


def ensure_writable_directory(path: Path, label: str) -> None:
    target = path.resolve(strict=False)
    try:
        target.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise RuntimeError(f"Unable to create {label} directory: {target}") from exc

    probe = target / ".sparkspawn-write-test"
    try:
        probe.write_text("ok", encoding="utf-8")
        probe.unlink()
    except OSError as exc:
        raise RuntimeError(f"{label} directory is not writable: {target}") from exc


def _detect_project_root() -> Path:
    return Path(__file__).resolve().parents[2]


def _resolve_default_ui_dir(project_root: Path) -> Optional[Path]:
    source_dist = project_root / "frontend" / "dist"
    if (source_dist / "index.html").exists():
        return source_dist.resolve(strict=False)

    packaged_dist = Path(__file__).resolve().parent / "ui_dist"
    if (packaged_dist / "index.html").exists():
        return packaged_dist.resolve(strict=False)

    return None


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
    default_value: Optional[Path],
) -> Optional[Path]:
    if cli_value is not None:
        candidate = _normalize_path(cli_value)
        return candidate
    if env_value:
        return _normalize_path(env_value)
    if default_value is None:
        return None
    return _normalize_path(default_value)


def _normalize_path(value: Path | str) -> Path:
    return Path(value).expanduser().resolve(strict=False)
