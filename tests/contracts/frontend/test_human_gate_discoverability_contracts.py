from __future__ import annotations

from tests.contracts.frontend._support.behavior_bridge import assert_frontend_behavior_contract_passed


def test_pending_human_gates_discoverable_in_execution_and_runs_views_item_10_1_01() -> None:
    assert_frontend_behavior_contract_passed("10.1.01")


def test_operator_can_answer_human_gate_prompts_without_leaving_ui_item_10_1_02() -> None:
    assert_frontend_behavior_contract_passed("10.1.02")
