from tests.contracts.frontend.frontend_behavior_runner import assert_frontend_behavior_contract_passed


def test_raw_to_structured_handoff_is_single_flight_item_11_3_01() -> None:
    assert_frontend_behavior_contract_passed("11.3.01")


def test_unsurfaced_data_survives_mixed_mode_editing_item_11_3_02() -> None:
    assert_frontend_behavior_contract_passed("11.3.02")


def test_raw_handoff_conflict_blocks_structured_mode_item_11_3_03() -> None:
    assert_frontend_behavior_contract_passed("11.3.03")
