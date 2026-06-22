# Parallel Join Authoring Polish

## Summary
Make `k_of_n` and `quorum` parallel joins first-class everywhere Spark authors, previews, validates, serializes, and documents flows. This ports the useful Metalshop behavior without adding new join policies.

## Key Changes
- Treat `join_policy` values as: `wait_all`, `first_success`, `k_of_n`, and `quorum`.
- For `k_of_n`, require `join_k` as an integer `>= 1` and `<=` the outgoing branch count.
- For `quorum`, allow optional `join_quorum`, defaulting to `0.5`; when present it must be finite and `0 < value <= 1`.
- Drop stale threshold attrs when policy changes:
  - non-`k_of_n` policies must not retain `join_k`
  - non-`quorum` policies must not retain `join_quorum`

## Implementation
- Backend:
  - Add explicit preview/API support for `join_k` and `join_quorum` in parallel node payloads.
  - Tighten DSL validation so invalid combinations fail before execution.
  - Mirror the same checks in the parallel handler so direct/programmatic execution cannot silently accept invalid threshold config.
- Frontend:
  - Add `join_k` and `join_quorum` to the canonical flow model as owned parallel attrs.
  - Serialize only the threshold attr that matches the active `join_policy`.
  - Add conditional authoring fields in the node inspector and canvas task-node editor:
    - K Threshold for `k_of_n`
    - Quorum Threshold for `quorum`
  - Clear inactive threshold attrs when changing policies.
- Docs/TODO:
  - Document the threshold attrs in the Attractor/Spark DOT authoring docs.
  - Mark or remove the TODO item once the behavior is implemented.

## Tests
- Add/adjust backend validator tests for:
  - valid `k_of_n` with `join_k`
  - missing/invalid/out-of-range `join_k`
  - valid `quorum` with and without `join_quorum`
  - invalid quorum values
  - stale threshold attrs on the wrong policy
- Add/adjust handler tests for matching runtime failure behavior.
- Add frontend tests for:
  - conditional threshold fields in inspector/canvas authoring
  - policy changes clearing stale attrs
  - canonical serialization emitting only the active threshold attr
- Run:
  - `uv run pytest -q -x --maxfail=1 tests/dsl/test_validator.py tests/handlers/test_parallel_handler.py tests/contracts/frontend/test_node_handler_roundtrip_contracts.py`
  - relevant frontend unit tests for editor/canvas/canonical flow model
  - final `uv run pytest -q`

## Assumptions
- This is a hard cleanup, not a compatibility pass: old raw DOT that used `k_of_n` without `join_k` should become invalid.
- We are not adding remote-worker, ingestion, primitive-chain, or unrelated Metalshop UI features.
