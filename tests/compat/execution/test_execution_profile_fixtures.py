from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from attractor.execution.config import load_execution_profile_config
from attractor.execution.errors import ExecutionProfileConfigError, ExecutionProfileSelectionError
from attractor.execution.resolution import resolve_execution_profile_by_id
from attractor.execution.settings_view import public_execution_placement_settings
from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


@dataclass(frozen=True)
class _Settings:
    config_dir: Path


def test_execution_profile_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
) -> None:
    manifests = [
        _profile_resolution_manifest(tmp_path),
        _local_container_unavailable_manifest(tmp_path),
    ]

    for manifest in manifests:
        _assert_runtime_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _profile_resolution_manifest(tmp_path: Path) -> dict[str, Any]:
    config_dir = tmp_path / "config"
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
[defaults]
execution_profile_id = "native-fast"

[profiles.native-fast]
label = "Native Fast"
mode = "native"
capabilities = ["filesystem", "subprocess"]

[profiles.container]
label = "Container"
mode = "local_container"
image = "spark-worker:compat"
enabled = false
capabilities = ["filesystem"]
[profiles.container.metadata]
worker = "spark-server worker run-node"
""",
        encoding="utf-8",
    )
    settings = _Settings(config_dir=config_dir)
    default_selection = resolve_execution_profile_by_id(settings)
    explicit_selection = resolve_execution_profile_by_id(settings, explicit_profile_id="native-fast")
    settings_view = public_execution_placement_settings(settings)
    return _manifest(
        fixture_id="runtime/execution-profile-resolution",
        scenario="execution_profile_resolution",
        input_payload=harness.normalize_path_tokens(
            {"config_dir": str(config_dir)},
            {"__CONFIG_DIR__": config_dir},
        ),
        observation=harness.normalize_path_tokens(
            {
                "default_selection": _selection_payload(default_selection),
                "explicit_selection": _selection_payload(explicit_selection),
                "settings_view": settings_view,
            },
            {"__CONFIG_DIR__": config_dir},
        ),
    )


def _local_container_unavailable_manifest(tmp_path: Path) -> dict[str, Any]:
    absent_config_settings = _Settings(config_dir=tmp_path / "absent-config")
    invalid_config_dir = tmp_path / "invalid-config"
    invalid_config_dir.mkdir(parents=True, exist_ok=True)
    (invalid_config_dir / "execution-profiles.toml").write_text(
        """
[profiles.container]
label = "Container"
mode = "local_container"
""",
        encoding="utf-8",
    )
    invalid_settings = _Settings(config_dir=invalid_config_dir)

    observations: dict[str, Any] = {}
    try:
        resolve_execution_profile_by_id(absent_config_settings, explicit_profile_id="container")
    except ExecutionProfileSelectionError as exc:
        observations["selected_without_config"] = _error_payload(exc)

    try:
        load_execution_profile_config(invalid_settings, explicit_profile_id="container")
    except ExecutionProfileConfigError as exc:
        observations["local_container_missing_image"] = {
            **_error_payload(exc),
            "field_errors": [
                {
                    "field": error.field,
                    "message": error.message,
                    "profile_id": error.profile_id,
                }
                for error in exc.field_errors
            ],
        }

    observations["public_settings_view"] = harness.normalize_path_tokens(
        public_execution_placement_settings(invalid_settings),
        {"__CONFIG_DIR__": invalid_config_dir},
    )
    return _manifest(
        fixture_id="runtime/execution-local-container-unavailable",
        scenario="execution_local_container_unavailable",
        input_payload=harness.normalize_path_tokens(
            {
                "absent_config_dir": str(absent_config_settings.config_dir),
                "invalid_config_dir": str(invalid_config_dir),
            },
            {
                "__ABSENT_CONFIG_DIR__": absent_config_settings.config_dir,
                "__CONFIG_DIR__": invalid_config_dir,
            },
        ),
        observation=harness.normalize_path_tokens(
            observations,
            {
                "__ABSENT_CONFIG_DIR__": absent_config_settings.config_dir,
                "__CONFIG_DIR__": invalid_config_dir,
            },
        ),
    )


def _selection_payload(selection: Any) -> dict[str, Any]:
    profile = selection.profile
    return {
        "selected_profile_id": selection.selected_profile_id,
        "selection_source": selection.selection_source,
        "profile": {
            "id": profile.id,
            "label": profile.label,
            "mode": profile.mode,
            "enabled": profile.enabled,
            "image": profile.image,
            "capabilities": list(profile.capabilities),
            "metadata": dict(profile.metadata),
        },
    }


def _error_payload(exc: Exception) -> dict[str, Any]:
    return {
        "error_type": exc.__class__.__name__,
        "message": str(exc),
    }


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": "compat-runtime-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I04,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-execution-profile-public-interfaces",
            "interfaces": [
                "attractor.execution.load_execution_profile_config",
                "attractor.execution.resolve_execution_profile_by_id",
                "attractor.execution.public_execution_placement_settings",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_runtime_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=ITEM_REQUIREMENTS,
        decision_ids=ITEM_DECISIONS,
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_runtime_manifest_matches_golden(manifest, expected)
