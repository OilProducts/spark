from __future__ import annotations

from pathlib import Path

import pytest

from harness import (
    HARNESS_INVOCATION,
    WORKFLOW_CASES,
    WORKFLOW_DIR,
    AcceptanceContext,
    collect_harness_with_uv,
    workflow_case_runner,
)


def test_every_markdown_workflow_asset_is_registered_in_harness() -> None:
    markdown_assets = {
        path.name
        for path in WORKFLOW_DIR.glob("*.md")
        if path.name != "README.md"
    }
    registered_assets = {case.markdown_asset for case in WORKFLOW_CASES}

    assert registered_assets == markdown_assets
    assert len({case.workflow_id for case in WORKFLOW_CASES}) == len(WORKFLOW_CASES)
    assert len({case.runner_name for case in WORKFLOW_CASES}) == len(WORKFLOW_CASES)
    for case in WORKFLOW_CASES:
        assert case.markdown_path.is_file()
        assert case.coverage


@pytest.mark.parametrize("case", WORKFLOW_CASES, ids=lambda case: case.workflow_id)
def test_agent_workflow_acceptance_case_executes(
    case,
    product_api_client,
    attractor_api_client,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ctx = AcceptanceContext(
        product=product_api_client,
        attractor=attractor_api_client,
        tmp_path=tmp_path,
        monkeypatch=monkeypatch,
    )

    result = workflow_case_runner(case)(ctx, case)

    assert result.workflow_id == case.workflow_id
    assert result.markdown_asset == case.markdown_asset
    assert set(case.coverage) <= result.outcomes
    assert result.evidence


def test_harness_collects_through_documented_uv_pytest_invocation() -> None:
    completed = collect_harness_with_uv()

    assert HARNESS_INVOCATION == ("uv", "run", "pytest", "-q", "tests/acceptance/agent-workflows")
    assert completed.returncode == 0, completed.stderr
    for case in WORKFLOW_CASES:
        assert case.workflow_id in completed.stdout
