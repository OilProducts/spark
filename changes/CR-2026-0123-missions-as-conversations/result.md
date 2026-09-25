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

The work is uncommitted in the working tree, alongside CR-2026-0121 and CR-2026-0122, and ships internally with missions.

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
