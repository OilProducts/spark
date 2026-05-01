from __future__ import annotations

from pathlib import Path

import pytest

from attractor.execution import (
    EXECUTION_MODES,
    ExecutionLaunchError,
    ExecutionProfile,
    ExecutionProfileConfigError,
    ExecutionProfileFieldError,
    ExecutionProfileSelectionError,
    WorkerProfile,
    build_launch_metadata,
    normalize_execution_mode,
)


def test_execution_modes_are_public_contract_values() -> None:
    assert EXECUTION_MODES == ("native", "local_container", "remote_worker")
    assert normalize_execution_mode(" LOCAL_CONTAINER ") == "local_container"
    with pytest.raises(ValueError, match="native, local_container, remote_worker"):
        normalize_execution_mode("container")


def test_execution_profile_defaults_to_native_without_configured_selection() -> None:
    profile = ExecutionProfile(mode="native")

    assert profile == ExecutionProfile(mode="native")
    assert profile.is_native is True
    assert profile.is_container is False


def test_execution_profile_image_is_metadata_on_selected_local_container_profile() -> None:
    profile = ExecutionProfile(id="local-dev", mode="local_container", image="spark-exec:latest")

    assert profile.mode == "local_container"
    assert profile.image == "spark-exec:latest"
    assert profile.is_container is True


def test_worker_and_execution_profile_models_are_typed_and_copy_metadata() -> None:
    capabilities = {"shell": True}
    metadata = {"region": "iad"}

    worker = WorkerProfile(
        id=" worker-a ",
        label=" Android Lab ",
        base_url=" https://worker.example:8765/ ",
        auth_token_env=" SPARK_WORKER_TOKEN ",
        capabilities=capabilities,
        metadata=metadata,
    )
    capabilities["shell"] = False
    metadata["region"] = "changed"

    assert worker.id == "worker-a"
    assert worker.label == "Android Lab"
    assert worker.base_url == "https://worker.example:8765"
    assert worker.auth_token_env == "SPARK_WORKER_TOKEN"
    assert worker.enabled is True
    assert worker.capabilities == {"shell": True}
    assert worker.metadata == {"region": "iad"}


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        ({"id": " "}, "worker id must be non-empty"),
        ({"label": ""}, "worker label must be non-empty"),
        ({"base_url": " "}, "worker base_url must be non-empty"),
        ({"auth_token_env": ""}, "worker auth_token_env must be non-empty"),
    ],
)
def test_worker_profile_rejects_empty_configured_worker_identity_fields(
    kwargs: dict[str, str],
    message: str,
) -> None:
    values = {
        "id": "worker-a",
        "label": "Worker A",
        "base_url": "https://worker.example:8765",
        "auth_token_env": "SPARK_WORKER_TOKEN",
    }
    values.update(kwargs)

    with pytest.raises(ValueError, match=message):
        WorkerProfile(**values)


def test_launch_metadata_snapshots_profile_fields_for_later_launch_integration() -> None:
    profile = ExecutionProfile(
        id=" remote-fast ",
        label=" Remote fast ",
        mode="remote_worker",
        worker_id=" worker-a ",
        image=" spark-worker:latest ",
        capabilities={"shell": True, "network": False},
        control_project_root=Path("/home/chris/projects"),
        worker_runtime_root=Path("/srv/runtime"),
    )

    assert profile.id == "remote-fast"
    assert profile.label == "Remote fast"
    assert profile.worker_id == "worker-a"
    assert profile.image == "spark-worker:latest"

    metadata = build_launch_metadata(
        profile,
        mapped_worker_project_path=Path("/srv/projects/acme"),
    ).as_dict()

    assert metadata == {
        "execution_mode": "remote_worker",
        "execution_profile_id": "remote-fast",
        "execution_container_image": "spark-worker:latest",
        "execution_control_project_root": "/home/chris/projects",
        "execution_mapped_project_path": "/srv/projects/acme",
        "execution_worker_runtime_root": "/srv/runtime",
        "execution_profile_capabilities": {"shell": True, "network": False},
    }


def test_execution_profile_rejects_empty_profile_identity_fields() -> None:
    with pytest.raises(ValueError, match="execution profile id must be non-empty"):
        ExecutionProfile(id=" ")

    with pytest.raises(ValueError, match="execution profile label must be non-empty"):
        ExecutionProfile(id="native", label="")

    with pytest.raises(ValueError, match="worker id must be non-empty"):
        ExecutionProfile(mode="remote_worker", worker_id="")


def test_execution_errors_expose_typed_profile_and_launch_failures() -> None:
    field_error = ExecutionProfileFieldError(
        field="profiles.remote.worker_id",
        message="unknown worker",
        profile_id="remote",
        worker_id="missing",
    )
    error = ExecutionProfileConfigError("invalid execution profile config", field_errors=(field_error,))

    assert isinstance(error, ValueError)
    assert error.field_errors == (field_error,)
    assert isinstance(ExecutionProfileSelectionError("missing profile"), ValueError)
    assert isinstance(ExecutionLaunchError("launch failed"), RuntimeError)
