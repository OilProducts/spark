---
id: CR-2026-0123-missions-as-conversations
title: Missions as conversations
status: completed
type: feature
changelog: internal
---

## Summary

A mission is now an objective plus a long-lived agent conversation. Start creates the conversation and sends the objective as the first turn. Run events and human replies go into the mission's inbox and reach the conversation as batched turns, with at most one turn in flight. Agents launch flows directly with `spark convo run-request`. The catalog launch policy still controls which flows they may launch, and the budget is checked at launch. Agents close missions with `spark mission close`. The mission frame is pinned as system instructions on every turn. Status is derived on read. The board is one list grouped Needs you, Running, Drafts, Closed, and the detail pane is the transcript.

Reactions, stages, hooks, holds, pause/resume, run signals and the events GET/CLI are removed.

The feature ships internally with missions, alongside CR-2026-0121 and CR-2026-0122.

## Validation

Run on 2026-09-25:

- `just test`: EXIT 0 (fmt check, all Rust tests, 82 frontend unit test files, frontend build). It was rerun against the final working tree when this result was recorded and exited 0 again.
- `npx vitest run src/features/missions`: 21 passed. When `MissionDetail` has no key, the new mission-switch test fails.
- `npm --prefix frontend run build`, then `npx playwright test e2e/smoke/missions-editor.spec.ts`: 4 passed (1440 and 390, light and dark).
- `tsc --noEmit` and eslint on the missions sources: clean.

## Shipped Changes

- **Mission service** (`crates/spark-workspace/src/missions.rs`, `conversations.rs`, `lib.rs`): conversation per mission, inbox delivery, launch with budget refusal and `context.spark_mission`, agent close, cancel, recovery, derived status, legacy load with done/review drafts archived. Mission conversations are hidden from Threads.
- **Pinned instructions** (`crates/spark-agent-adapter`): `AGENT_INSTRUCTIONS_METADATA_KEY`. Claude Code gets `--append-system-prompt-file` on every spawn. Codex gets `developerInstructions` on `thread/start` and `thread/resume`. The workspace chat frame uses the same mechanism.
- **HTTP and CLI** (`crates/spark-http`, `crates/spark-cli`): `run-request` launches directly. Adds `mission close`. Removes events, pause and resume.
- **Assets**: removed `missions/react.yaml` and updated `spark-operations.md`.
- **Frontend** (`frontend/src/features/missions`): grouped list, transcript detail, header menu and Budget form.
  - `MissionDetail` is keyed by mission id, so reply drafts, the Budget form and errors do not carry over between missions.
  - `useMissionConversation` keeps each snapshot with its conversation id, drops out-of-order responses and surfaces fetch errors.
  - Control actions (Start, Cancel, Close) show the server's 409 reason.
- **Tests**: service, adapter, CLI and HTTP route contracts; `MissionsPanel.test.tsx`, which adds the mission-switch, control-409 and inline flow-launch cases; and the missions smoke spec, which now covers a needs-you detail pane.

## Known limits

- A roster entry stays `waiting` until its run ends, so a run that resumes after its gate is answered keeps the mission in Needs you (ponytail in `missions.rs`).
- The transcript refetches the full snapshot on each live event on top of the 2s poll while running. Debounce it or subscribe it to the live stream if that load becomes a problem.
- `.gitignore` does not un-ignore this change request.


## Cancellation and test-race repair (2026-09-27)

For mission `mission-c5505392-f161-43f0-ac0d-12e8572513dd`:

- The HTTP mission contract gates its agent until the Start response's `running` assertion, then releases it and explicitly checks `needs_you`. Existing assertions remain.
- The workspace cancel contract waits for its tool to enter a release-file gate, asserts mission closure and persisted `cancel_requested`, then releases the tool and checks terminal cancellation. The gate expires after 600 polls if the test fails.
- `finalize_completed` rereads the persisted record before setting completion, propagates read errors, and delegates `cancel_requested` to the existing canceled finalizer. This covers the dead-end, successful-exit, and unsatisfied-goal-gate completion callers.
- A channel-gated runtime contract covers all three callers: cancellation is accepted while the last task is blocked, then the released executor must return canceled, persist the canceled record and result, emit canceled runtime status and the existing cancellation `PipelineFailed` event, and never emit `PipelineCompleted`.

Verification logs are retained at `/tmp/mission-c5505392-verification/`:

- `regression-before.log`: expected failure, exit 101; old completion returned `completed` instead of `canceled` after accepting cancellation.
- `regression-after.log`: regression passed for all three completion callers.
- `http-targeted.log` and `workspace-targeted.log`: both repaired mission contracts passed.
- Targeted commands used `cargo test --workspace --all-features --test contracts <test-name>`.
- Full validation: three sequential foreground `just test` runs all exited 0 (`just-test-1.log`: 221.4s; `just-test-2.log`: 200.9s; `just-test-3.log`: 204.0s). Each passed formatting, all Rust tests, 83 frontend test files / 628 tests, and the frontend production build. No unexpected test failures occurred.
- `git diff --check`: passed. Log SHA-256 hashes and the final diff hash are retained in `manifest.json` beside the logs.

Branch: `spark/implement-change/run-18d9491766f288a8`. Base commit: `700ec614c11b0959dbb2c5ab0133243a086e01b3`. The reviewed repair is committed on this branch. `/tmp/mission-c5505392-verification/committed-handoff.json` records the exact commit, committed file hashes, and association with the original verification manifest. Only this result documentation changed after review; executable source and tests match the verified diff, so the existing verification remains applicable. The original manifest and logs are preserved unchanged.
