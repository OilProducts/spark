---
id: CR-2026-0053-parallel-join-authoring-polish
title: Parallel Join Authoring Polish
status: completed
type: feature
changelog: public
---

## Summary

Implemented first-class `k_of_n` and `quorum` parallel join authoring and validation. Parallel nodes now expose, serialize, validate, preview, and execute `join_k` and `join_quorum` only for their matching `join_policy`, and stale threshold attrs are cleared or rejected.

## Validation

- `uv run pytest -q -x --maxfail=1 tests/dsl/test_validator.py tests/handlers/test_parallel_handler.py tests/contracts/frontend/test_node_handler_roundtrip_contracts.py` passed: 93 tests.
- `npm run test:unit -- InspectorAndNodeAuthoring.test.tsx TaskNode.test.tsx` passed: 2 files, 11 tests. The run emitted React `act(...)` warnings from the exercised component tests.
- `uv run pytest -q` passed: 1943 tests, 26 skipped.

## Shipped Changes

- Backend preview/API payloads now include `join_k` and `join_quorum` for parallel nodes.
- DSL validation and the parallel handler now enforce supported join policies, required/ranged `join_k`, optional/ranged `join_quorum`, and stale-threshold rejection.
- Frontend canonical flow serialization, the sidebar inspector, and the canvas task-node editor now own the threshold attrs, show conditional controls, and drop inactive threshold attrs on policy changes.
- Tests cover backend validation, runtime handler rejection, frontend serialization, and conditional authoring behavior.
- Spark DOT authoring docs, the Attractor spec, and `TODO.md` now document the implemented parallel threshold attrs.
