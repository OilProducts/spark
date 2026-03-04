from tests.contracts.frontend.frontend_behavior_runner import assert_frontend_behavior_contract_passed


def test_required_ui_api_endpoints_have_runtime_coverage_item_12_1_01() -> None:
    assert_frontend_behavior_contract_passed("12.1.01")
