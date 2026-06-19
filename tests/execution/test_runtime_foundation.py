from __future__ import annotations

import pytest

from attractor.execution import (
    EXECUTION_MODES,
    ExecutionLaunchError,
    ExecutionProfile,
    ExecutionProfileConfigError,
    ExecutionProfileFieldError,
    ExecutionProfileSelectionError,
    build_launch_metadata,
    normalize_execution_mode,
)


def test_execution_modes_are_public_contract_values() -> None:
    assert EXECUTION_MODES == ("native", "local_container")
    assert normalize_execution_mode(" LOCAL_CONTAINER ") == "local_container"
    with pytest.raises(ValueError, match="native, local_container"):
        normalize_execution_mode("container")
    with pytest.raises(ValueError, match="native, local_container"):
        normalize_execution_mode("remote_worker")


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


def test_execution_profile_models_are_typed_and_copy_metadata() -> None:
    capabilities = {"shell": True}
    metadata = {"region": "iad"}

    profile = ExecutionProfile(
        id=" local-dev ",
        label=" Local Dev ",
        mode="local_container",
        image=" spark-exec:latest ",
        capabilities=capabilities,
        metadata=metadata,
    )
    capabilities["shell"] = False
    metadata["region"] = "changed"

    assert profile.id == "local-dev"
    assert profile.label == "Local Dev"
    assert profile.image == "spark-exec:latest"
    assert profile.capabilities == {"shell": True}
    assert profile.metadata == {"region": "iad"}


def test_launch_metadata_snapshots_profile_fields_for_later_launch_integration() -> None:
    profile = ExecutionProfile(
        id=" local-dev ",
        label=" Local Dev ",
        mode="local_container",
        image=" spark-exec:latest ",
        capabilities={"shell": True, "network": False},
    )

    assert profile.id == "local-dev"
    assert profile.label == "Local Dev"
    assert profile.image == "spark-exec:latest"

    metadata = build_launch_metadata(profile).as_dict()

    assert metadata == {
        "execution_mode": "local_container",
        "execution_profile_id": "local-dev",
        "execution_container_image": "spark-exec:latest",
        "execution_profile_capabilities": {"shell": True, "network": False},
    }


def test_execution_profile_rejects_empty_profile_identity_fields() -> None:
    with pytest.raises(ValueError, match="execution profile id must be non-empty"):
        ExecutionProfile(id=" ")

    with pytest.raises(ValueError, match="execution profile label must be non-empty"):
        ExecutionProfile(id="native", label="")

    with pytest.raises(ValueError, match="native, local_container"):
        ExecutionProfile(mode="remote_worker")


def test_execution_errors_expose_typed_profile_and_launch_failures() -> None:
    field_error = ExecutionProfileFieldError(
        field="profiles.bad.mode",
        message="invalid mode",
        profile_id="bad",
    )
    error = ExecutionProfileConfigError("invalid execution profile config", field_errors=(field_error,))

    assert isinstance(error, ValueError)
    assert error.field_errors == (field_error,)
    assert isinstance(ExecutionProfileSelectionError("missing profile"), ValueError)
    assert isinstance(ExecutionLaunchError("launch failed"), RuntimeError)
