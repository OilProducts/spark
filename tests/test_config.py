from __future__ import annotations

from pathlib import Path

from spark_common.settings import resolve_settings


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
