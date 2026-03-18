from __future__ import annotations

from pathlib import Path

from attractor.config import resolve_settings


def test_resolve_settings_defaults_flows_dir_to_repo_flows_when_running_from_git_repo() -> None:
    settings = resolve_settings(env={})

    expected_project_root = Path(__file__).resolve().parents[1]
    expected_data_dir = Path.home() / ".sparkspawn"

    assert settings.project_root == expected_project_root
    assert settings.flows_dir == expected_data_dir / "flows"
