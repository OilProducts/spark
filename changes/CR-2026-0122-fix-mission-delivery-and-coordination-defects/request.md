# Fix mission delivery and coordination defects

## Summary

Correct defects found in an audit of the uncommitted CR-2026-0121 missions implementation. The most serious ones leave a mission stuck: a run ends but the mission never receives its terminal event, so the roster entry stays `running`, holds a `concurrent_runs` slot, and the mission shows Running forever. Others lose directive actions, hide failures behind budget waits, keep canceled missions in the attention feed, and discard unsaved edits in the UI.

Out of scope: restricting what `POST /workspace/api/missions/{id}/events` accepts (intentional), and whether assistants may start missions without run approval (a separate policy change).

## 1. Deliver every terminal run event

- `deliver_run_events` (`crates/spark-workspace/src/missions.rs`) suppresses a terminal event while `result.json` is `pending`. Several terminal paths never replace the pending result written by `create_run`: restart recovery (`AttractorApiService::fail_recovery`), `RuntimeControls::mark_canceled`, and launch failure. Every path that sets a terminal run status must leave a non-pending result, so the mission receives exactly one terminal event.
- Result writes on the failure path (`executor.rs`, `failed_run_result`), the cancel path, and the launch-failure path (`attractor-api`) do not notify run-event observers; only `RunStore::materialize_result` does. A failed run whose result summary takes seconds is therefore delivered only after a server restart. Move the notification into the shared result-write path so every terminal result write notifies, and remove the success-only special case.
- Restart recovery must deliver terminal events for owned runs in either state above.

## 2. Keep directive actions when a launch fails

- In `apply`, a failed `launch` sets an Attention hold with no actions, which drops the rest of the directive. `reaction_events` was already cleared, so nothing can recover them. Keep the actions after the failed launch in the hold. When a human clears the attention (Resume or a message), apply those actions without retrying the failed launch.

## 3. Do not let a budget wait replace an attention hold

- A budget Waiting hold set in `apply` or `maybe_react` overwrites an existing Attention hold. When the budget frees, `reduce` takes the Waiting hold and the failure disappears without a human seeing it. An Attention hold must survive until a human clears it. Actions deferred for budget while attention is set must still run after it is cleared and the budget allows.

## 4. Clear attention for human-closed missions

- `settle` maps every closed mission that is not `done` to `attention`, and the attention feed lists it until it is archived or moved to Done. A mission a human canceled or closed needs no further attention: show it as idle, with the close reason. Missions closed `failed` or `canceled` by a reaction or hook stay in `attention`, as CR-2026-0121 specified.

## 5. Mission detail UI

- `MissionExecution.tsx` resets the hooks/budget YAML whenever `mission.revision` changes, which throws away unsaved edits when unrelated activity arrives. Reset the text only when it has no unsaved edits. Otherwise keep the edits and let the existing revision-conflict handling apply when they are saved.
- The Events list refetches only when `cursor`, the roster length, or `updated_at` change, so an event that leaves the record unchanged (such as a message sent while paused) never appears. Show newly appended events, and fetch incrementally with the existing `after` parameter rather than downloading the whole inbox each time.

## 6. Keep mission delivery off the hot path for unrelated runs

- `deliver_run_events` runs on every coalesced publish for every run. It scans `runs_dir` to find the run root and reads the record before it knows whether the run belongs to a mission. For mission runs that have ever written `context.mission.signal`, every later publish also takes the project-wide mission lock and rereads the inbox. Return early, cheaply, for runs that no mission owns. Take the mission lock only when there is a new signal value or a terminal or waiting status not yet delivered.

## Verification

- Service or runtime tests: a mission-owned run failed by restart recovery, canceled through `mark_canceled`, or failing to launch produces exactly one terminal event and frees its roster slot; a failed run whose result is written after `pipeline_failed` is delivered without a restart.
- Service tests: a directive whose first launch fails keeps its later `set_state` and `close` and applies them after Resume; a budget wait arising while attention is set does not clear the attention, and the deferred launch runs once attention is cleared and budget frees; a human cancel settles to idle and leaves the attention feed; a reaction-closed `failed` mission stays in attention.
- Frontend tests: unsaved hooks YAML survives a live revision bump; a message sent while paused appears in Events.
- Run the mission contracts, the affected attractor-runtime and attractor-api suites, the HTTP, CLI, and live contracts touched by CR-2026-0121, the frontend unit tests, the frontend build, and `just test`.
