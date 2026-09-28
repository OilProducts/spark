---
id: CR-2026-0131-triggers-deliver-to-missions
title: Triggers deliver events to missions
status: completed
type: feature
changelog: public
---

## Summary

Triggers can deliver external events to missions through the existing inbox and turn batching. Mission agents can declare why they are waiting for an outside event; idle missions with an enabled targeting trigger then appear in Running with that reason instead of requiring user attention.

## Validation

- `just test`: passed (exit 0), including formatting, all Rust suites, 635 frontend tests across 83 files, and the production frontend build. Follow-up gate log: `/tmp/cr131-close-gate-rerun.log`.
- Mission integration coverage exercises schedule, poll, webhook, and flow-event delivery, agent wake-up and active-turn batching, complete inbox payload retention, quoted/truncated rendering, draft/closed no-op history, target validation, close-time disabling and activity, waiting transitions, attention-feed omission, wait clearing, and rejection of externally posted trigger events.
- Close synchronization regression: `cargo test --workspace --all-features --test contracts mission_contracts` passed (21 mission tests; `/tmp/cr131-close-rust.log`). Human close, cancel, and assistant close each publish exactly the changed triggers, with revisions matching storage; already-disabled and unrelated triggers are untouched, and repeated closure emits no duplicates.
- The initial follow-up full gate exposed a time-dependent test assertion comparing recalculated schedule `next_run_at` values. The regression now checks persisted revision, enabled state, identity, source type, and target action directly.
- Close UI regression: `npm --prefix frontend run test:unit -- src/features/missions/__tests__/MissionsPanel.test.tsx src/features/triggers/__tests__/TriggersPanel.test.tsx` passed (35 tests; `/tmp/cr131-close-ui.log`). Tests load Triggers first, switch to mission detail, then close/cancel or receive agent closure events. Both mounted displays change from Enabled to Disabled through live updates, preserving their DOM nodes without manual refresh.
- CLI contracts exercise mission trigger creation and update through real HTTP routes and `mission wait` reason persistence and request construction.
- UI contracts exercise the open-mission action picker, mission creation payload, waiting display, targeting-trigger navigation, and parsing actions without a flow name.
- `git diff --check`: passed. Non-failing React act warnings and frontend bundle-size advisories remain.

## Shipped Changes

- Mission trigger actions persist `mission_id` and `project_path` without a flow target. Create/update validate the mission under the same project lock used by closure.
- Activations append `trigger.fired` events through the mission service with trigger identity, source type, and the full source payload. Draft and closed targets use existing explanatory no-op history.
- Closing a mission disables enabled targeting triggers and identifies their names and IDs in close activity. The shared close/cancel path publishes each saved trigger through the existing runtime and `trigger.upsert` envelope, using the new persisted revision and normal trigger serialization. This also synchronizes mission detail and the cached Triggers view when the agent closes the mission.
- Agent prompts quote JSON payload data, bound excerpts to 4,096 characters, and report the full serialized byte size. Mission instructions treat payloads as external data, never instructions, and distinguish outside-event waiting from missing-information questions.
- `spark mission wait --project <path> --id <id> --reason <text>` retains the reason until a turn starts. Waiting derives from an idle, open mission with a reason and an enabled targeting trigger; other missions retain existing Needs you behavior.
- Triggers offers Deliver to mission with a project mission picker. Mission detail lists targeting trigger names, source types, enabled state, and navigation to Triggers, using existing UI structures.
- Outbound hooks and domain playbooks remain out of scope.
