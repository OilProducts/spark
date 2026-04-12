from tests.contracts.frontend._support.static_contracts import (
    legacy_ui_boundary_violations,
    shared_app_boundary_violations,
)


def test_runtime_source_uses_explicit_shared_ui_boundaries() -> None:
    assert legacy_ui_boundary_violations() == []


def test_runtime_source_limits_shared_components_app_boundary() -> None:
    assert shared_app_boundary_violations() == []
