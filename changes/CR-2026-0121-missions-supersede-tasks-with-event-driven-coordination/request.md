# Missions: replace tasks with an event-driven work coordinator

## Summary

Rename tasks to **missions** and give them execution. A mission is a project-scoped record of an intended outcome that may own any number of concurrently executing runs and reacts to events from them until it closes itself or a human closes it. A mission that has not started is exactly a task today: the six-stage board, drafts, conflicts, and activity history carry over unchanged.

The coordination model is the actor model. One mission has one durable inbox, processes events one batch at a time, and launches runs that execute in parallel. Deterministic **hooks** handle mechanical reactions; a **reaction run** supplies judgment when a hook says so or nothing matches. Only reaction runs write the mission's state summary, so there is one writer.

This is slice one of three. Slice two adds a trigger action mode that delivers webhook, schedule, poll, and flow events into a mission. Slice three adds outbound hooks and domain reaction flows (research program, PR shepherd).

## Why not the alternatives

- **Keep tasks and add a linked mission entity.** CR-2026-0110 linked runs to tasks ad hoc and CR-2026-0114 removed it. A one-to-one link with stage-sync rules is that design again. A mission record is a strict superset of a task record, so one entity costs less than two plus a link.
- **Mission as one long-running run with an internal loop.** Fragile across restarts and multi-day spans, and it cannot own parallel runs without the manager loop, which cannot carry execution profiles to children (see CR-2026-0092).
- **Single-flight steps with fork/join directives.** Duplicates what a run can do with child runs and adds partial-completion bookkeeping to the reducer for no case the actor model does not already cover.

## Record and lifecycle

- Fields are the task fields plus mission fields. Retain `title`, `description` (the objective), `stage`, `archived`, identity, project scope, timestamps, `revision`, and attributed activity. Add:
  - `state`: replaceable current-state summary, markdown, written only by reactions.
  - `reaction_flow`: logical flow name, default `missions/react.yaml`.
  - `hooks`: ordered list, see below.
  - `budget`: `{ concurrent_runs, total_runs, reactions }`.
  - `runs`: roster of owned runs `{ run_id, label, role: work|reaction, launched_at, launched_by_event, status }`.
  - `execution`: `{ substate: idle|running|reasoning|waiting|attention, reason }`.
  - `cursor`: last processed inbox sequence.
  - `closed`: `{ status: done|failed|canceled, reason, at }` when set.
- Ids are `mission-<uuid>`. Existing `task-<uuid>` ids remain valid; an existing `tasks/` directory is read as `missions/` on first access with no other migration.
- Stages stay the manual lifecycle: Backlog, Planning, Ready, In progress, Review, Done. Moving a card launches nothing (unchanged from CR-2026-0114). Execution lives inside In progress:
  - **Start** is an explicit action. It sets stage to In progress and posts `mission.started`.
  - `execution.substate` reports what is happening: `running` while work runs are in flight, `reasoning` while a reaction is in flight, `waiting` when a reaction is blocked on a human gate or a budget is exhausted, `attention` when a reaction failed or a run failed with no matching hook.
  - A reaction or hook that closes the mission as `done` moves the stage to Review. A human moves it to Done. `failed` and `canceled` leave the stage at In progress with `attention` set; a human decides.
  - Pause stops inbox processing and leaves in-flight runs alone. Resume processes the backlog. Cancel cancels in-flight owned runs and closes as `canceled`.
- Storage: `SPARK_HOME/workspace/projects/<project-id>/missions/<mission-id>.json` for the record, and `missions/<mission-id>/events.jsonl` for the inbox, using the existing atomic write and append helpers. Project deletion removes both.

## Events and delivery

- Event shape: `{ seq, id, at, kind, source, payload }`. `source` names a run id, an actor, or a trigger id. `id` is caller-supplied for idempotent delivery; duplicates are ignored.
- Kinds in this slice: `mission.started`, `run.completed`, `run.failed`, `run.canceled`, `run.waiting`, `run.signal`, `human.message`. The kind namespace is open for slice two.
- One delivery route: `POST /workspace/api/missions/{id}/events`. The UI and server-side sources use it; the trigger action mode in slice two will too.
- Run terminal and waiting events are posted by the server from the existing terminal-run publish hook, keyed by `run_id` plus status so repeated publishes of a terminal run deliver once. A terminal event is not posted while the run's result is still `pending`.
- `run.signal` is how a run speaks mid-flight. A node writes `context.mission.signal` (a JSON value); the run event observer turns that write into an event with the value as payload. Runs never call the HTTP API. Containers keep having no control surface.
- Processing is sequential per mission under the existing per-project lock. The reducer reads events after `cursor`, applies hooks in order, advances the cursor, and writes the record atomically. Events that arrive while a reaction is in flight queue and reach the next reaction as one batch.

## Hooks and reactions

- A hook is `{ on: kind, label?: string, status?: string, do: action }`. `label` matches the owned run's label; `status` matches a terminal status. Matching is exact; first match wins; no match means `reason`. No expression language in this slice.
- Actions: `launch { flow_name, label, context }`, `close { status, reason }`, `ignore`, `reason`. `launch` runs through the existing workspace launch path with the project's default execution profile and injects `context.spark_mission { mission_id, label, role }` the same way triggers inject `context.spark_trigger`. The launch is recorded in the roster before the response returns.
- A **reaction** is one run of `reaction_flow`, role `reaction`, launched with `context.mission { id, title, objective, state, runs, events }` where `events` is the pending batch. It ends by writing `context.mission.directive = { actions: [...] }`. Directive actions are `launch`, `set_state { markdown }`, and `close`. The reducer reads the directive from the checkpoint when the reaction completes and applies the actions in order. A reaction that needs a decision blocks on the existing human gate or agent clarification; the mission shows `waiting` and work runs continue.
- At most one reaction in flight per mission. A failed or canceled reaction sets `attention` and is not retried automatically; a human message or Resume triggers the next reaction with the accumulated batch.
- Ship one built-in reaction flow, `missions/react.yaml`: an agent reads objective, state, roster, and batch, decides, and writes the directive. It is deliberately generic.

## Budgets and recovery

- Defaults: 4 concurrent runs, 25 total runs, 10 reactions. Editable per mission. A `launch` that would exceed a limit is not performed; the mission enters `waiting` with the limit named. Raising the budget resumes processing. Reactions count against total runs as well as reactions.
- Restart recovery runs beside `recover_interrupted_runs`: for every roster entry not terminal, read the run record; post any missing terminal event; then resume inbox processing for missions with unprocessed events.

## API, CLI, and live updates

- Replace `/workspace/api/tasks` with `/workspace/api/missions`: list, create, get, patch (revision-checked, same mutation shape), plus `POST {id}/events`, `GET {id}/events?after=`, and `POST {id}/start|pause|resume|cancel`. All keep `?project_path`.
- Replace `spark task …` with `spark mission list|get|create|update|start|send|events`. `send` posts a `human.message`.
- Add a `mission` resource kind to the live stream (`mission.upsert` envelope) and subscribe from the frontend when the Missions view is active. Drop the 15 s board poll.
- The attention inbox includes missions in `waiting` or `attention`.
- Update the bundled assistant guide from tasks to missions, including how to start one and send it a message.

## UI

- Rename the Tasks tab to Missions. Keep the board, lanes, cards, search, archive toggle, drafts, conflicts, read/edit modes, and focus behavior from CR-2026-0114 and CR-2026-0115.
- Cards in In progress add a compact execution chip (substate) and an in-flight run count. Cards in other stages are unchanged.
- The detail pane keeps the current read and edit views and adds sections that appear once a mission has started: **State** (rendered markdown), **Runs** (roster with status; opening one selects it in Runs), **Events** (inbox tail with a compose box that sends a `human.message`), **Hooks and budget** (edited as YAML text in this slice), and **Controls** (Start, Pause, Resume, Cancel, Close). Start is the only control shown before a mission starts.
- Use existing theme tokens and shared controls. No new palette, board framework, or drag-and-drop.

## Verification

- Service tests: record superset loads existing task files; revision conflicts; idempotent event delivery; cursor advance; hook matching precedence and the `reason` default; directive application order; single writer of `state`; one reaction in flight with batching; budget refusal and resume; cancel cancels owned runs; project isolation.
- Runtime tests: `context.mission.signal` write produces exactly one `run.signal`; terminal publish of the same run posts one event; no terminal event while the result is `pending`; recovery posts missing terminal events after a simulated restart.
- Contract test end to end: start a mission whose reaction launches two work runs with labels; a hook launches a follow-up when the first completes; the second fails with no hook, a reaction runs with both events in one batch and closes the mission; stage lands in Review and the roster, inbox, and activity agree.
- HTTP, CLI, and live tests for the renamed routes and commands, the events route, controls, and `mission.upsert` delivery.
- Frontend tests: existing board and editor suites adapted to the rename; execution chip; started-mission sections; sending a message; controls; opening a run. Browser smoke in light and dark at wide and narrow widths with empty, populated, and started boards.
- Run affected Rust and frontend suites, the frontend build, and `just test`.

## Defaults

- Tasks are renamed, not kept alongside. No compatibility routes or CLI aliases.
- No trigger action mode, webhook delivery, outbound hooks, or schedule-driven reactions in this slice.
- No cost cap; add after the parent-run usage rollup in TODO.md lands.
- Hooks match exactly and are edited as text. A structured hook editor and an expression language are deferred until real hook sets show what they need.
- Reactions batch events; a batch can be stale by the length of one reaction. Accepted for now.
- Runs signal through context writes only. No HTTP control surface for runs.
