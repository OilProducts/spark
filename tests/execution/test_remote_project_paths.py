from __future__ import annotations

from pathlib import Path

import pytest

from attractor.execution import (
    ExecutionLaunchError,
    ExecutionProfile,
    build_launch_metadata,
    map_remote_project_path,
)


def _remote_profile() -> ExecutionProfile:
    return ExecutionProfile(
        id="remote-fast",
        mode="remote_worker",
        worker_id="worker-a",
        image="spark-worker:latest",
        control_project_root=Path("/home/chris/projects"),
        worker_project_root=Path("/srv/projects"),
        worker_runtime_root=Path("/srv/runtime"),
        capabilities=("shell", "network"),
    )


def test_remote_project_mapping_preserves_relative_suffix() -> None:
    mapped = map_remote_project_path(
        _remote_profile(),
        Path("/home/chris/projects/acme/service"),
    )

    assert mapped == Path("/srv/projects/acme/service")


def test_remote_project_mapping_rejects_outside_control_root() -> None:
    with pytest.raises(ExecutionLaunchError, match="outside remote control_project_root"):
        map_remote_project_path(_remote_profile(), Path("/home/chris/other/acme"))


def test_remote_project_mapping_rejects_string_prefix_false_positive() -> None:
    with pytest.raises(ExecutionLaunchError, match="outside remote control_project_root"):
        map_remote_project_path(_remote_profile(), Path("/home/chris/projects-other/acme"))


def test_remote_mapping_and_metadata_helpers_do_not_mutate_filesystem(tmp_path: Path) -> None:
    control_root = tmp_path / "control"
    project_path = control_root / "acme"
    worker_root = tmp_path / "worker"
    runtime_root = tmp_path / "runtime"
    profile = ExecutionProfile(
        id="remote-fast",
        mode="remote_worker",
        worker_id="worker-a",
        image="spark-worker:latest",
        control_project_root=control_root,
        worker_project_root=worker_root,
        worker_runtime_root=runtime_root,
        capabilities={"shell": True},
    )

    metadata = build_launch_metadata(profile, control_project_path=project_path).as_dict()

    assert metadata["execution_mapped_project_path"] == str(worker_root / "acme")
    assert metadata["execution_worker_runtime_root"] == str(runtime_root)
    assert metadata["execution_profile_capabilities"] == {"shell": True}
    assert not control_root.exists()
    assert not project_path.exists()
    assert not worker_root.exists()
    assert not runtime_root.exists()


def test_launch_metadata_snapshots_local_container_image_and_capabilities() -> None:
    profile = ExecutionProfile(
        id="local-dev",
        mode="local_container",
        image="spark-exec:latest",
        capabilities={"network": False},
    )

    assert build_launch_metadata(profile).as_dict() == {
        "execution_mode": "local_container",
        "execution_profile_id": "local-dev",
        "execution_container_image": "spark-exec:latest",
        "execution_profile_capabilities": {"network": False},
    }
