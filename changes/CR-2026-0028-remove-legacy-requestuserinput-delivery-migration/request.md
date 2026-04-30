# Remove Legacy `request_user_input` Delivery Migration

## Summary
Hard cut over by deleting the special-case migration for `request_user_input.delivery_status: "pending_delivery"` without adding new deprecated-field validation. Clean current conversation files keep loading, and legacy fields are simply ignored as unknown payload keys instead of being normalized to `expired`.

## Key Changes
- Remove the `legacy_delivery_status` parameter from `_normalize_request_user_input_status`.
- Remove the `delivery_status` lookup from `RequestUserInputRecord.from_dict`.
- Delete the old test that asserts `answered + delivery_status: pending_delivery` normalizes to `expired`.
- Keep current `pending` / `answered` / `expired` status normalization unchanged.
- Keep existing output behavior where `RequestUserInputRecord.to_dict()` never emits `delivery_status` or `delivered_at`.
- Keep `ConversationState.normalize_request_user_input_state()` because it still maintains current `expired` segment/turn consistency.

## Tests
- Remove `test_request_user_input_legacy_pending_delivery_state_normalizes_to_expired`.
- Do not add a replacement deprecated-field rejection test.
- Keep existing observable behavior tests for:
  - pending request input segments
  - answered request input segments
  - expired request input segments
  - no `delivery_status`/`delivered_at` emitted after answer submission
  - answer submission without a live session expiring the request

## Verification
- Run targeted project chat tests:
  `uv run pytest -q -x --maxfail=1 tests/api/test_project_chat.py`
- Run the required full suite before completion:
  `uv run pytest -q`

## Assumptions
- Hard cutover means removing the compatibility behavior and related tests, not adding a new compatibility guard.
- Existing clean schema-4 conversation files must continue loading.
- If an old file still has `status: "answered"` plus `delivery_status: "pending_delivery"`, it will now load as `answered` because `delivery_status` is ignored.
