---
id: CR-2026-0028-remove-legacy-requestuserinput-delivery-migration
title: Remove Legacy request_user_input Delivery Migration
status: completed
type: refactor
changelog: internal
---

## Summary
Removed the legacy `request_user_input.delivery_status: "pending_delivery"` compatibility path. `RequestUserInputRecord.from_dict()` now normalizes status only from the current `status` field, so legacy `delivery_status` data is ignored as an unknown payload key instead of converting an answered request to expired.

## Validation
The change runtime reports the implementation and evaluation stages completed successfully using the requested validation commands:

- `uv run pytest -q -x --maxfail=1 tests/api/test_project_chat.py`
- `uv run pytest -q`

## Shipped Changes
- Updated `src/spark/workspace/conversations/models.py` to remove the `legacy_delivery_status` parameter and `delivery_status` lookup from request-user-input status normalization.
- Updated `tests/api/test_project_chat.py` to remove the old legacy `pending_delivery` migration regression test while leaving current pending, answered, and expired request-user-input behavior covered by the existing suite.
