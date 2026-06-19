from __future__ import annotations

from pathlib import Path

import pytest

from attractor.execution import (
    ExecutionProfileConfigError,
    ExecutionProfileSelectionError,
    build_launch_metadata,
    load_execution_profile_config,
    resolve_execution_profile_by_id,
)
from spark.settings import resolve_settings


def test_missing_execution_profiles_config_synthesizes_native_default_without_selected_profile(
    tmp_path: Path,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})

    graph = load_execution_profile_config(settings)

    assert graph.synthesized_native_default is True
    assert set(graph.profiles) == {"native"}
    assert graph.profiles["native"].mode == "native"
    assert graph.profiles["native"].enabled is True


@pytest.mark.parametrize(
    ("selection_kwargs", "selected_profile_id"),
    [
        ({"explicit_profile_id": "local-dev"}, "local-dev"),
        ({"explicit_profile_id": "native"}, "native"),
        ({"project_default_profile_id": "local-dev"}, "local-dev"),
        ({"project_default_profile_id": "native"}, "native"),
        ({"spark_default_profile_id": "local-dev"}, "local-dev"),
        ({"spark_default_profile_id": "native"}, "native"),
    ],
)
def test_missing_execution_profiles_config_does_not_fallback_when_profile_selected(
    tmp_path: Path,
    selection_kwargs: dict[str, str],
    selected_profile_id: str,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})

    with pytest.raises(ExecutionProfileSelectionError, match=selected_profile_id):
        load_execution_profile_config(settings, **selection_kwargs)


def test_execution_profiles_toml_normalizes_profiles_and_ignores_workers_table(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [workers.worker-a]
        label = " Worker A "
        enabled = true
        base_url = " https://worker.example:8765/ "
        auth_token_env = " SPARK_WORKER_TOKEN "

        [profiles.native-dev]
        mode = " native "
        label = " Native Dev "

        [profiles.local-dev]
        label = " Local Dev "
        mode = "LOCAL_CONTAINER"
        image = " spark:latest "
        capabilities = ["network", "gpu"]
        metadata = { region = "iad" }
        """,
        encoding="utf-8",
    )

    graph = load_execution_profile_config(settings, explicit_profile_id="local-dev")

    assert graph.synthesized_native_default is False
    assert set(graph.profiles) == {"native-dev", "local-dev"}
    assert graph.profiles["native-dev"].enabled is True
    assert graph.profiles["native-dev"].label == "Native Dev"
    local = graph.profiles["local-dev"]
    assert local.mode == "local_container"
    assert local.image == "spark:latest"
    assert local.capabilities == ("network", "gpu")
    assert local.metadata == {"region": "iad"}
    metadata = build_launch_metadata(local).as_dict()
    assert metadata["execution_profile_capabilities"] == ["network", "gpu"]
    assert metadata["execution_container_image"] == "spark:latest"


@pytest.mark.parametrize(
    ("body", "field", "profile_id", "message"),
    [
        (
            """
            [profiles.local]
            mode = "local_container"
            label = "Local"
            """,
            "profiles.local.image",
            "local",
            "image is required for local_container profiles",
        ),
        (
            """
            [profiles.bad]
            label = "Bad"
            mode = "container"
            """,
            "profiles.bad.mode",
            "bad",
            "execution mode must be one of: native, local_container",
        ),
        (
            """
            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            image = "spark-worker:latest"
            """,
            "profiles.remote.mode",
            "remote",
            "execution mode must be one of: native, local_container",
        ),
    ],
)
def test_execution_profiles_config_errors_identify_failing_profile_field(
    tmp_path: Path,
    body: str,
    field: str,
    profile_id: str,
    message: str,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(body, encoding="utf-8")

    with pytest.raises(ExecutionProfileConfigError) as exc_info:
        load_execution_profile_config(settings)

    assert any(
        error.field == field
        and error.profile_id == profile_id
        and error.message == message
        for error in exc_info.value.field_errors
    )


@pytest.mark.parametrize(
    ("body", "field", "profile_id"),
    [
        (
            """
            [profiles.native]
            mode = "native"
            """,
            "profiles.native.label",
            "native",
        ),
        (
            """
            [profiles.native]
            mode = "native"
            label = "Native"
            capabilities = { docker = true }
            """,
            "profiles.native.capabilities",
            "native",
        ),
        (
            """
            [profiles.native]
            mode = "native"
            label = "Native"
            capabilities = ["docker", ""]
            """,
            "profiles.native.capabilities[1]",
            "native",
        ),
        (
            """
            [profiles.local]
            mode = "local_container"
            label = "Local"
            image = 123
            """,
            "profiles.local.image",
            "local",
        ),
    ],
)
def test_execution_profiles_config_rejects_non_contract_profile_fields(
    tmp_path: Path,
    body: str,
    field: str,
    profile_id: str,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(body, encoding="utf-8")

    with pytest.raises(ExecutionProfileConfigError) as exc_info:
        load_execution_profile_config(settings)

    assert any(
        error.field == field and error.profile_id == profile_id
        for error in exc_info.value.field_errors
    )


def test_selected_disabled_profile_fails_without_silent_fallback(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.native]
        mode = "native"
        label = "Native"

        [profiles.paused]
        mode = "native"
        label = "Paused"
        enabled = false
        """,
        encoding="utf-8",
    )

    with pytest.raises(ExecutionProfileSelectionError, match="paused"):
        load_execution_profile_config(settings, explicit_profile_id="paused")


def test_disabled_profiles_remain_visible_but_unselectable(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [workers.paused-worker]
        label = "Ignored Worker"
        enabled = false

        [profiles.native]
        mode = "native"
        label = "Native"

        [profiles.paused]
        mode = "native"
        label = "Paused"
        enabled = false
        """,
        encoding="utf-8",
    )

    graph = load_execution_profile_config(settings)

    assert graph.profiles["paused"].enabled is False
    with pytest.raises(ExecutionProfileSelectionError, match="paused"):
        resolve_execution_profile_by_id(settings, explicit_profile_id="paused")


def test_execution_profile_id_resolution_precedence(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [defaults]
        execution_profile_id = "spark-default"

        [profiles.explicit]
        mode = "native"
        label = "Explicit"

        [profiles.project-default]
        mode = "native"
        label = "Project Default"

        [profiles.spark-default]
        mode = "native"
        label = "Spark Default"
        """,
        encoding="utf-8",
    )

    assert (
        resolve_execution_profile_by_id(
            settings,
            explicit_profile_id="explicit",
            project_default_profile_id="project-default",
        ).profile.id
        == "explicit"
    )
    assert (
        resolve_execution_profile_by_id(
            settings,
            project_default_profile_id="project-default",
        ).profile.id
        == "project-default"
    )
    assert resolve_execution_profile_by_id(settings).profile.id == "spark-default"


def test_execution_profile_id_resolution_preserves_native_without_configured_selection(
    tmp_path: Path,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})

    selection = resolve_execution_profile_by_id(settings)

    assert selection.profile.mode == "native"
    assert selection.profile.id == "native"
    assert selection.selection_source == "implementation_default"


def test_execution_profile_id_resolution_does_not_select_configured_native_without_selection(
    tmp_path: Path,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.native]
        mode = "native"
        label = "Configured Native"
        enabled = false
        """,
        encoding="utf-8",
    )

    selection = resolve_execution_profile_by_id(settings)

    assert selection.selection_source == "implementation_default"
    assert selection.selected_profile_id == "native"
    assert selection.profile.id == "native"
    assert selection.profile.label == "Native"
