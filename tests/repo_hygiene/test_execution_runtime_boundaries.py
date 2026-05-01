from __future__ import annotations

import importlib
from typing import Any

import attractor.api.server as server
from attractor.execution import EXECUTION_MODES
import spark.chat.response_parsing
import spark.chat.service
import spark.chat.session
import spark.cli
import spark.workspace.api
import spark.workspace.storage
OWNED_RUNTIME_CLASS_NAMES = {
    "ExecutionProfile",
    "WorkerProfile",
    "ExecutionProfileError",
    "ExecutionProfileConfigError",
    "ExecutionProfileSelectionError",
    "ExecutionLaunchError",
}


def test_execution_runtime_package_owns_profile_and_error_models() -> None:
    for class_name in OWNED_RUNTIME_CLASS_NAMES:
        exported = getattr(importlib.import_module("attractor.execution"), class_name)
        assert exported.__module__.startswith("attractor.execution.")
        assert _declaring_module(exported).__name__.startswith("attractor.execution.")


def test_integration_surfaces_import_without_owning_runtime_model_contracts() -> None:
    integration_modules = (
        server,
        spark.workspace.api,
        spark.workspace.storage,
        spark.chat.response_parsing,
        spark.chat.service,
        spark.chat.session,
        spark.cli,
    )

    for module in integration_modules:
        for class_name in OWNED_RUNTIME_CLASS_NAMES:
            exported = getattr(module, class_name, None)
            if exported is not None:
                assert getattr(exported, "__module__", "").startswith("attractor.execution.")


def test_module_boundary_execution_modes_are_foundation_values() -> None:
    assert EXECUTION_MODES == ("native", "local_container", "remote_worker")


def test_worker_api_routes_are_not_mounted_on_control_plane_app() -> None:
    route_paths = {
        str(getattr(route, "path", ""))
        for route in server.attractor_app.routes
    }

    assert not any("/workers" in path or "/execution-workers" in path for path in route_paths)


def _declaring_module(value: Any) -> Any:
    return importlib.import_module(value.__module__)
