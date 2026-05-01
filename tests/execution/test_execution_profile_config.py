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
    assert graph.workers == {}
    assert set(graph.profiles) == {"native"}
    assert graph.profiles["native"].mode == "native"
    assert graph.profiles["native"].enabled is True


@pytest.mark.parametrize(
    ("selection_kwargs", "selected_profile_id"),
    [
        ({"explicit_profile_id": "remote-fast"}, "remote-fast"),
        ({"explicit_profile_id": "native"}, "native"),
        ({"project_default_profile_id": "remote-fast"}, "remote-fast"),
        ({"project_default_profile_id": "native"}, "native"),
        ({"spark_default_profile_id": "remote-fast"}, "remote-fast"),
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


def test_execution_profiles_toml_normalizes_workers_and_profiles(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [workers.worker-a]
        label = " Worker A "
        enabled = true
        base_url = " https://worker.example:8765/ "
        auth_token_env = " SPARK_WORKER_TOKEN "
        capabilities = [" shell ", "docker"]
        metadata = { region = "iad" }

        [profiles.native-dev]
        mode = " native "
        label = " Native Dev "

        [profiles.container]
        label = " Container "
        mode = "LOCAL_CONTAINER"
        image = " spark:latest "

        [profiles.remote-fast]
        mode = "remote_worker"
        label = " Remote Fast "
        worker = " worker-a "
        image = " spark-worker:latest "
        control_project_root = "/home/chris/projects"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        capabilities = ["network", "gpu"]
        """,
        encoding="utf-8",
    )

    graph = load_execution_profile_config(settings, explicit_profile_id="remote-fast")

    assert graph.synthesized_native_default is False
    assert set(graph.workers) == {"worker-a"}
    assert graph.workers["worker-a"].label == "Worker A"
    assert graph.workers["worker-a"].base_url == "https://worker.example:8765"
    assert graph.workers["worker-a"].auth_token_env == "SPARK_WORKER_TOKEN"
    assert graph.workers["worker-a"].capabilities == ("shell", "docker")
    assert set(graph.profiles) == {"native-dev", "container", "remote-fast"}
    assert graph.profiles["native-dev"].enabled is True
    assert graph.profiles["native-dev"].label == "Native Dev"
    assert graph.profiles["container"].mode == "local_container"
    assert graph.profiles["container"].image == "spark:latest"
    remote = graph.profiles["remote-fast"]
    assert remote.worker_id == "worker-a"
    assert remote.control_project_root == Path("/home/chris/projects")
    assert remote.worker_project_root == Path("/srv/projects")
    assert remote.worker_runtime_root == Path("/srv/runtime")
    assert remote.capabilities == ("network", "gpu")
    metadata = build_launch_metadata(
        remote,
        control_project_path=Path("/home/chris/projects/acme/service"),
    ).as_dict()
    assert metadata["execution_profile_capabilities"] == ["network", "gpu"]
    assert metadata["execution_mapped_project_path"] == "/srv/projects/acme/service"


@pytest.mark.parametrize(
    ("body", "field", "profile_id", "worker_id"),
    [
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.local]
            mode = "local_container"
            label = "Local"
            """,
            "profiles.local.image",
            "local",
            None,
        ),
        (
            """
            [profiles.bad]
            label = "Bad"
            mode = "container"
            """,
            "profiles.bad.mode",
            "bad",
            None,
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            worker = "missing"
            image = "spark-worker:latest"
            control_project_root = "/control"
            worker_project_root = "/worker/project"
            worker_runtime_root = "/worker/runtime"
            """,
            "profiles.remote.worker",
            "remote",
            "missing",
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"
            """,
            "workers.worker-a.enabled",
            None,
            "worker-a",
        ),
    ],
)
def test_execution_profiles_config_errors_identify_failing_id_and_field(
    tmp_path: Path,
    body: str,
    field: str,
    profile_id: str | None,
    worker_id: str | None,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(body, encoding="utf-8")

    with pytest.raises(ExecutionProfileConfigError) as exc_info:
        load_execution_profile_config(settings)

    assert any(
        error.field == field
        and error.profile_id == profile_id
        and error.worker_id == worker_id
        for error in exc_info.value.field_errors
    )


@pytest.mark.parametrize(
    ("body", "field", "profile_id", "worker_id"),
    [
        (
            """
            [workers.worker-a]
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"
            """,
            "workers.worker-a.label",
            None,
            "worker-a",
        ),
        (
            """
            [profiles.native]
            mode = "native"
            """,
            "profiles.native.label",
            "native",
            None,
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            worker_id = "worker-a"
            image = "spark-worker:latest"
            control_project_root = "/control"
            worker_project_root = "/worker/project"
            worker_runtime_root = "/worker/runtime"
            """,
            "profiles.remote.worker_id",
            "remote",
            "worker-a",
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
            None,
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
            None,
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
            None,
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            worker = "worker-a"
            image = "spark-worker:latest"
            control_project_root = 123
            worker_project_root = "/worker/project"
            worker_runtime_root = "/worker/runtime"
            """,
            "profiles.remote.control_project_root",
            "remote",
            None,
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            worker = 123
            image = "spark-worker:latest"
            control_project_root = "/control"
            worker_project_root = false
            worker_runtime_root = "/worker/runtime"
            """,
            "profiles.remote.worker_project_root",
            "remote",
            None,
        ),
        (
            """
            [workers.worker-a]
            label = "Worker A"
            enabled = true
            base_url = "https://worker.example"
            auth_token_env = "TOKEN"

            [profiles.remote]
            mode = "remote_worker"
            label = "Remote"
            worker = "worker-a"
            image = "spark-worker:latest"
            control_project_root = "/control"
            worker_project_root = "/worker/project"
            worker_runtime_root = 123
            """,
            "profiles.remote.worker_runtime_root",
            "remote",
            None,
        ),
    ],
)
def test_execution_profiles_config_rejects_non_contract_toml_fields(
    tmp_path: Path,
    body: str,
    field: str,
    profile_id: str | None,
    worker_id: str | None,
) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(body, encoding="utf-8")

    with pytest.raises(ExecutionProfileConfigError) as exc_info:
        load_execution_profile_config(settings)

    assert any(
        error.field == field
        and error.profile_id == profile_id
        and error.worker_id == worker_id
        for error in exc_info.value.field_errors
    )


def test_remote_worker_profile_rejects_disabled_worker(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [workers.worker-a]
        label = "Worker A"
        enabled = false
        base_url = "https://worker.example"
        auth_token_env = "TOKEN"

        [profiles.remote]
        mode = "remote_worker"
        label = "Remote"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "/control"
        worker_project_root = "/worker/project"
        worker_runtime_root = "/worker/runtime"
        """,
        encoding="utf-8",
    )

    with pytest.raises(ExecutionProfileConfigError) as exc_info:
        load_execution_profile_config(settings)

    assert exc_info.value.field_errors[0].profile_id == "remote"
    assert exc_info.value.field_errors[0].worker_id == "worker-a"
    assert "disabled" in exc_info.value.field_errors[0].message


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


def test_disabled_profiles_and_workers_remain_visible_but_unselectable(tmp_path: Path) -> None:
    settings = resolve_settings(env={"SPARK_HOME": str(tmp_path)})
    settings.config_dir.mkdir(parents=True)
    (settings.config_dir / "execution-profiles.toml").write_text(
        """
        [workers.paused-worker]
        label = "Paused Worker"
        enabled = false
        base_url = "https://worker.example"
        auth_token_env = "TOKEN"

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

    assert graph.workers["paused-worker"].enabled is False
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
    assert selection.profile.enabled is True
