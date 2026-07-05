# Manager-Loop Telemetry Ingestion

## Summary
Complete `stack.manager_loop` telemetry ingestion beyond the current runtime context snapshot so Rust-backed Spark observes concrete child-run runtime state and records it into the runtime-owned `context.stack.child.*` snapshot used by manager-loop stop conditions, steering decisions, events, and diagnostics.

## Background
The Rust rewrite migration record still lists `manager_loop_telemetry_ingestion` as an open policy gap. Manager-loop runtime behavior already supports child launch, observation cycles, child intervention, autostart, stop-condition evaluation, and first-class authoring fields. The remaining gap is that observation is too shallow: it preserves a latest-child context snapshot, but it does not fully ingest concrete child-run telemetry such as active stage, lifecycle status, terminal outcome, failure reason, retry/progress indicators, and artifact/event summary data from the authoritative child run state.

The Attractor spec defines manager-loop observation as ambiguous progress telemetry. This change must preserve that contract: telemetry ingestion updates observable context, but it must not infer failure or trigger steering from elapsed time, unchanged active stage, missing artifacts, or long-running work. Automatic steering remains driven only by concrete child failure context.

## Required Behavior
- During manager-loop `observe` cycles, ingest authoritative child-run telemetry for the active linked child run into runtime-owned `context.stack.child.*` keys.
- Preserve existing snapshot fields and add missing concrete fields where supported by current Python behavior and Rust runtime state, including:
  - child run id and lifecycle status
  - active/current stage or node
  - terminal outcome and outcome reason code/message
  - failure reason when the child has failed or explicitly reported failure context
  - retry/progress counters when available from child checkpoints or status records
  - artifact/event summary counts or latest observable timestamps when available
- Keep `context.stack.child.*` runtime-owned. Authored flows may read these keys, but manager-loop runtime code owns clearing/updating them.
- Preserve autostart semantics:
  - `running` child state suppresses a new autostarted child.
  - terminal child state from a previous manager invocation is stale when `stack.child_autostart=true`.
  - fresh autostart clears runtime-owned child snapshot fields before writing the new child result.
- Preserve automatic steering semantics:
  - telemetry alone is not failure context.
  - elapsed time, unchanged active stage, missing artifacts, and long-running work do not trigger automatic steering.
  - concrete failure context still comes from child failure reason, outcome reason fields, or failure outcome/status.
- Emit or retain manager-loop observability evidence so tests can distinguish telemetry ingestion from steering/intervention.
- Update migration/TODO/coverage records so `manager_loop_telemetry_ingestion` is no longer an open policy gap when implementation is complete.

## Non-Goals
- Do not redesign manager-loop steering policy.
- Do not add stall/progress heuristics.
- Do not change manager-loop authoring UI; that was closed by `CR-2026-0057`.
- Do not build the agent-driven acceptance workflow harness.
- Do not address subgraph/scoped-defaults structured UI authoring.
- Do not run the final post-gap spec/API drift audit except to keep touched migration evidence coherent.

## Suggested Target Paths
- `crates/attractor-runtime/src/manager_loop.rs`
- `crates/attractor-runtime/src/checkpoints.rs`
- `crates/attractor-runtime/src/records.rs`
- `crates/attractor-runtime/src/events.rs`
- `crates/attractor-runtime/tests/core_handler_contracts.rs`
- `crates/attractor-runtime/tests/context_routing_contracts.rs`
- `src/attractor/handlers/builtin/manager_loop.py`
- `tests/handlers/test_manager_loop_handler.py`
- `tests/api/test_manager_loop_pipeline_api.py`
- `specs/attractor-spec.md`
- `TODO.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`

## Tests
- Add focused runtime tests proving an observe cycle updates `context.stack.child.*` from a concrete child run/checkpoint/status record.
- Add regression coverage proving telemetry-only progress signals do not trigger automatic steering.
- Add coverage for terminal child outcome/failure ingestion and stop-condition behavior.
- Add or update Python compatibility tests if Python retained modules define the oracle behavior for manager-loop telemetry.
- Update migration/coverage repo-hygiene tests so `manager_loop_telemetry_ingestion` is closed only with concrete implementation evidence.
- Run focused validation for touched runtime, Python compatibility, and repo-hygiene tests.
- Final verification: `uv run pytest -q`.

## Acceptance Criteria
- A manager-loop parent observing an active child run records concrete child status/stage/outcome/failure telemetry into `context.stack.child.*`.
- Child terminal success/failure is reflected from authoritative child run state and still routes according to existing manager-loop rules.
- Automatic steering remains conservative and is not triggered by ambiguous progress telemetry alone.
- `manager_loop_telemetry_ingestion` is removed from open policy gaps and recorded as closed by implementation evidence.
- The change result documents shipped behavior and validation.
