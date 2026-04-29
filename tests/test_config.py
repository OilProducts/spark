from __future__ import annotations

import os
from pathlib import Path

from attractor.api.runtime_paths import resolve_runtime_paths, validate_runtime_paths
from spark.settings import resolve_settings


def test_resolve_settings_defaults_flows_dir_to_repo_flows_when_running_from_git_repo() -> None:
    settings = resolve_settings(env={})

    expected_project_root = Path(__file__).resolve().parents[1]
    expected_data_dir = (Path.home() / ".spark").resolve(strict=False)

    assert settings.project_root == expected_project_root
    assert settings.flows_dir == expected_data_dir / "flows"


def test_resolve_settings_defaults_flows_dir_under_resolved_data_dir_from_env(tmp_path: Path) -> None:
    data_dir = tmp_path / "spark-home"

    settings = resolve_settings(env={"SPARK_HOME": str(data_dir)})

    assert settings.data_dir == data_dir.resolve(strict=False)
    assert settings.flows_dir == data_dir.resolve(strict=False) / "flows"


def test_resolve_settings_defaults_to_no_project_roots() -> None:
    settings = resolve_settings(env={})

    assert settings.project_roots == ()


def test_resolve_settings_parses_single_project_root(tmp_path: Path) -> None:
    projects_dir = tmp_path / "projects"

    settings = resolve_settings(env={"SPARK_PROJECT_ROOTS": str(projects_dir)})

    assert settings.project_roots == (projects_dir.resolve(strict=False),)


def test_resolve_settings_parses_multiple_project_roots(tmp_path: Path) -> None:
    primary_dir = tmp_path / "primary"
    secondary_dir = tmp_path / "secondary"

    settings = resolve_settings(
        env={"SPARK_PROJECT_ROOTS": os.pathsep.join([str(primary_dir), "", str(secondary_dir)])}
    )

    assert settings.project_roots == (
        primary_dir.resolve(strict=False),
        secondary_dir.resolve(strict=False),
    )


def test_resolve_attractor_runtime_paths_requires_explicit_runtime_runs_and_flows(tmp_path: Path) -> None:
    paths = resolve_runtime_paths(
        runtime_dir=tmp_path / "runtime",
        runs_dir=tmp_path / "runs",
        flows_dir=tmp_path / "flows",
    )

    assert paths.runtime_dir == (tmp_path / "runtime").resolve(strict=False)
    assert paths.runs_dir == (tmp_path / "runs").resolve(strict=False)
    assert paths.flows_dir == (tmp_path / "flows").resolve(strict=False)


def test_validate_attractor_runtime_paths_creates_required_directories(tmp_path: Path) -> None:
    paths = resolve_runtime_paths(
        runtime_dir=tmp_path / "runtime",
        runs_dir=tmp_path / "runs",
        flows_dir=tmp_path / "flows",
    )

    validate_runtime_paths(paths)

    assert paths.runtime_dir.is_dir()
    assert paths.runs_dir.is_dir()
    assert paths.flows_dir.is_dir()
