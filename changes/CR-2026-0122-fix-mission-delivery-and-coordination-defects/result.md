---
id: CR-2026-0122-fix-mission-delivery-and-coordination-defects
title: Fix mission delivery and coordination defects
status: completed
type: bugfix
changelog: internal
---

## Summary

Fixed the defects found in the audit of the CR-2026-0121 missions implementation. Missions no longer get stuck `running` when a run ends through restart recovery, cancel, or launch failure. Directive actions survive a failed launch. Budget waits no longer overwrite attention holds. Missions a human closes settle to idle. The mission detail UI keeps unsaved edits and shows newly appended events.

The work is uncommitted in the working tree, alongside the uncommitted CR-2026-0121 missions implementation it corrects. Both are shipped internally with missions, so changelog is `internal`.

## Validation

Run on 2026-09-25 while recording this result:

- `cargo test -p spark-workspace --test contracts mission`: 15 passed. The run includes `a_failed_directive_launch_keeps_its_remaining_actions_until_resume`, `a_budget_wait_never_replaces_attention_and_runs_after_it_clears`, `runs_ended_without_an_executor_deliver_exactly_one_terminal_event`, `terminal_events_wait_for_results_and_recovery_posts_missing_events`, and `reaction_or_hook_failures_stay_in_attention_until_moved_to_done`.
- `cargo test -p attractor-runtime -p attractor-api`: all suites passed, 0 failed. The runtime storage contract and the attractor-api recovery contracts now assert that recovery and orphaned cancel leave a terminal result.
- `npx vitest run src/features/missions`: 29 passed. The run includes "keeps unsaved hooks YAML across a live revision bump" and "shows a message sent while paused, fetching only newer events".

Not rerun while recording this result: the HTTP, CLI, and live SSE contracts, the full frontend unit suite, the frontend build, and `just test`. The request's verification list names all of these.

## Shipped Changes

- **Terminal result writes notify observers** (`crates/attractor-runtime/src/store.rs`, `results.rs`, `executor.rs`, `controls.rs`, `lib.rs`): `RunStore::write_result` now notifies run-event observers. `materialize_run_result` became `build_run_result`, and callers persist its result through the store, which removes the success-only notification. `mark_canceled` now writes a `canceled` result through a new `canceled_run_result` helper.
- **Recovery and launch failure** (`crates/attractor-api/src/lib.rs`): `fail_recovery` replaces the pending result with a failed one. The launch-failure path writes its result through `store.write_result`, so it notifies observers.
- **Mission coordinator** (`crates/spark-workspace/src/missions.rs`):
  - Run ownership is cached per run, so runs that no mission owns return early without scanning `runs_dir` again.
  - Recovery delivers terminal events for runs whose result is still pending.
  - A failed launch keeps the rest of the directive in its attention hold.
  - A budget wait no longer replaces an attention hold.
  - `settle` shows missions a human closed as idle. Missions a reaction or hook closed as `failed` or `canceled` stay in attention.
- **Mission detail UI** (`frontend/src/features/missions/MissionExecution.tsx`): The hooks and budget YAML follows the server only when it has no unsaved edits. Events are fetched incrementally with `?after=<seq>`.
- **Tests**:
  - `crates/spark-workspace/tests/contracts/mission_contracts.rs`
  - `crates/attractor-runtime/tests/contracts/storage_contracts.rs`
  - `crates/attractor-api/tests/contracts/run_recovery_contracts.rs`
  - `frontend/src/features/missions/__tests__/MissionsPanel.test.tsx`
