from __future__ import annotations

import importlib
import importlib.util
from pathlib import Path
import subprocess
from typing import Any

import attractor.api.server as server
import attractor.execution as execution
import spark.chat.response_parsing
import spark.chat.service
import spark.chat.session
import spark.cli
import spark.server_cli
import spark.workspace.api
import spark.workspace.storage

REPO_ROOT = Path(__file__).resolve().parents[2]

OWNED_RUNTIME_CLASS_NAMES = {
    "ExecutionProfile",
    "ExecutionLaunchMetadata",
    "ExecutionProfileGraph",
    "ExecutionProfileSelection",
    "ExecutionProfileError",
    "ExecutionProfileConfigError",
    "ExecutionProfileSelectionError",
    "ExecutionLaunchError",
    "ExecutionProtocolError",
}

OWNED_RUNTIME_FUNCTION_NAMES = {
    "build_launch_metadata",
    "load_execution_profile_config",
    "normalize_execution_mode",
    "public_execution_placement_settings",
    "resolve_execution_profile_by_id",
    "seed_execution_profile_context",
}

OWNED_RUNTIME_MODULES = (
    "attractor.execution.config",
    "attractor.execution.context",
    "attractor.execution.errors",
    "attractor.execution.models",
    "attractor.execution.modes",
    "attractor.execution.resolution",
    "attractor.execution.settings_view",
)

INTEGRATION_MODULES = (
    server,
    spark.workspace.api,
    spark.workspace.storage,
    spark.chat.response_parsing,
    spark.chat.service,
    spark.chat.session,
    spark.cli,
    spark.server_cli,
)

REMOTE_EXECUTION_MODULES = (
    "attractor.execution.metadata",
    "attractor.execution.paths",
    "attractor.execution.remote_client",
    "attractor.execution.remote_runner",
    "attractor.execution.worker_app",
    "attractor.execution.worker_bridge",
    "attractor.execution.worker_models",
    "attractor.execution.worker_runtime",
    "attractor.execution.worker_state",
)


def test_execution_runtime_package_owns_profile_and_error_models() -> None:
    for class_name in OWNED_RUNTIME_CLASS_NAMES:
        exported = getattr(execution, class_name)
        assert exported.__module__.startswith("attractor.execution.")
        assert _declaring_module(exported).__name__.startswith("attractor.execution.")


def test_execution_runtime_package_owns_loading_metadata_transport_worker_and_bridge_exports() -> None:
    for function_name in OWNED_RUNTIME_FUNCTION_NAMES:
        exported = getattr(execution, function_name)
        assert exported.__module__.startswith("attractor.execution.")

    for module_name in OWNED_RUNTIME_MODULES:
        module = importlib.import_module(module_name)
        assert _module_path(module).is_relative_to(REPO_ROOT / "src" / "attractor" / "execution")


def test_integration_surfaces_import_without_owning_runtime_contracts() -> None:
    owned_names = OWNED_RUNTIME_CLASS_NAMES | OWNED_RUNTIME_FUNCTION_NAMES

    for module in INTEGRATION_MODULES:
        for public_name in owned_names:
            exported = getattr(module, public_name, None)
            if exported is not None:
                assert getattr(exported, "__module__", "").startswith("attractor.execution.")


def test_module_boundary_execution_modes_are_foundation_values() -> None:
    assert execution.EXECUTION_MODES == ("native", "local_container")


def test_remote_worker_api_routes_are_not_mounted_on_control_plane_app() -> None:
    route_paths = {
        str(getattr(route, "path", ""))
        for route in server.attractor_app.routes
    }

    for worker_route_prefix in ("/v1/health", "/v1/worker-info", "/v1/runs"):
        assert not any(path == worker_route_prefix or path.startswith(f"{worker_route_prefix}/") for path in route_paths)


def test_remote_worker_runtime_modules_are_removed() -> None:
    for module_name in REMOTE_EXECUTION_MODULES:
        assert importlib.util.find_spec(module_name) is None


def test_execution_container_module_remains_run_node_compatibility_glue() -> None:
    execution_container = importlib.import_module("attractor.handlers.execution_container")

    assert hasattr(execution_container, "run_worker_node")
    assert not hasattr(execution_container, "create_worker_app")
    assert not hasattr(execution_container, "WorkerState")
    assert not hasattr(execution_container, "RemoteWorkerClient")


def test_no_separate_remote_worker_delivery_package_or_committed_runtime_state() -> None:
    tracked_paths = _git_ls_files()

    forbidden_package_roots = (
        "src/remote_execution/",
        "src/remote_worker/",
        "src/spark_remote_worker/",
        "remote_execution/",
        "remote_worker/",
    )
    forbidden_runtime_roots = (
        ".spark/",
        ".planflow/",
        "artifacts/",
        "src/spark/ui_dist/",
    )
    forbidden_runtime_filenames = {
        "validation-result.json",
    }

    assert not any(path.startswith(forbidden_package_roots) for path in tracked_paths)
    assert not any(path.startswith(forbidden_runtime_roots) for path in tracked_paths)
    assert not any(Path(path).name in forbidden_runtime_filenames for path in tracked_paths)


def test_no_accidental_top_level_remote_worker_import_package() -> None:
    for package_name in ("remote_execution", "remote_worker", "spark_remote_worker"):
        spec = importlib.util.find_spec(package_name)
        if spec is not None and spec.origin is not None:
            assert not Path(spec.origin).resolve(strict=False).is_relative_to(REPO_ROOT)


def _declaring_module(value: Any) -> Any:
    return importlib.import_module(value.__module__)


def _module_path(module: Any) -> Path:
    module_file = getattr(module, "__file__", None)
    assert module_file is not None
    return Path(module_file).resolve(strict=False)


def _git_ls_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]
