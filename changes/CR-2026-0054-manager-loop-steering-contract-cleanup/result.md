---
id: CR-2026-0054-manager-loop-steering-contract-cleanup
title: Manager-Loop Steering Contract Cleanup
status: completed
type: feature
changelog: public
---

## Summary
Implemented the bounded automatic manager steering contract. Manager-loop steering now remains tied to concrete child failure context, tracks automatic attempts by child run, target node, and failure reason during a manager invocation, and records skipped intervention results when the same failure point has already been attempted.

## Validation
- `uv run pytest -q`: 1946 passed, 26 skipped.
- `uv run pytest -q -x --maxfail=1 tests/handlers/test_manager_loop_handler.py tests/api/test_pipeline_steer_endpoint.py`: 34 passed.

## Shipped Changes
- Updated `src/attractor/handlers/builtin/manager_loop.py` to enforce the automatic steering repeat guard, preserve cooldown as a throttle, and include child run, target node, failure reason, and skipped-result details in manager intervention artifacts/events.
- Expanded manager-loop handler coverage for first automatic intervention, repeated-key suppression, new target nodes, new failure reasons, cooldown spacing, no-failure behavior, and delivered intervention metadata.
- Added API steering coverage showing repeated human/API steering remains unaffected by the automatic repeat guard.
- Rewrote the stale progress-aware steering TODO/spec language and documented the supported manager-loop authoring surface, including `manager.steer_cooldown` and the automatic repeat guard, in `TODO.md`, `specs/attractor-spec.md`, and `src/spark/guides/dot-authoring.md`.
