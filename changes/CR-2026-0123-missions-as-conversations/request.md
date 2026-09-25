# Missions as conversations

## Summary

Replace the mission's reaction machinery with a conversation. A mission is an objective plus a long-lived agent conversation that owns the mission's runs. Run events are delivered to that conversation automatically as new turns, so nobody forwards results and the agent never polls. The transcript is the mission's audit trail, and you answer the mission by typing into it.

Also: agents launch flows without an approval gate, the mission objective is pinned through the system prompt instead of conversation history, and the Missions board becomes a single list grouped by derived status. Remove everything that existed only to serve reactions.

## Why

The reaction model runs a fresh one-shot flow for every batch of events. It has no memory beyond an overwritten state summary, its reasoning is buried in each reaction run's files, you cannot talk to it, and customizing it means writing a flow. A conversation gives memory, a readable transcript, and a reply box, and Spark already has one.

## Mission record

- Keep: identity, project scope, `title`, `description` (the objective), `archived`, timestamps, `revision`, activity, `runs` roster, `budget`, `started_at`, `closed`, and the inbox (`events.jsonl`) with its `cursor`.
- Add `conversation_id`: the mission's conversation, created on Start.
- Roster entries are `{ run_id, flow_name, summary, launched_at, status }`.
- Budget is `{ concurrent_runs, total_runs }`, defaults 4 and 25. There is no limit on turns.
- Loading a record written before this change ignores removed fields. An unstarted record whose old `stage` is `done` or `review` is archived on load, so finished tasks do not reappear as drafts.

## Conversation and delivery

- Start creates the mission's conversation and sends its first turn: the objective and an instruction to begin.
- Mission conversations use the normal conversation store and turn execution. They are hidden from the project's Threads list.
- The inbox stays the single, durable delivery queue. It carries `run.completed`, `run.failed`, `run.canceled`, `run.waiting`, and `human.message`.
- When no turn is in flight, pending inbox events are sent as the next turn: one turn per batch, oldest first, each event rendered as a short line with the run's flow, summary, status, and a pointer to the run. Events that arrive during a turn wait for the next one. At most one turn is in flight per mission.
- A message you type in the transcript is posted as `human.message` and delivered the same way.
- Cancel cancels in-flight runs and closes the mission.
- Restart recovery keeps working: post missing terminal events for owned runs, then deliver anything pending.

## Launching and closing

- Remove the approval gate for agent-launched flows. `spark convo run-request` launches the flow immediately and returns the run id. The flow catalog's launch policy still decides which flows agents may launch.
- A launch from a mission's conversation adds a roster entry and injects `context.spark_mission`, so its events come back to the mission.
- Budgets are checked at launch. A launch over budget is refused with the limit named; the agent sees the refusal as the command's output.
- The agent closes the mission with `spark mission close --status done|failed|canceled --reason <text>`. Add `close` to the CLI; the route exists.
- Update the assistant frame and `spark-operations.md`: launches are direct, and approval language is removed.

## Pinned instructions

- The mission frame (Spark control surface, the mission's id, and the objective) is passed as system instructions on every turn, not in conversation history, so compaction cannot drop it and it is not repeated in the transcript.
- Claude Code: `--append-system-prompt-file` on every spawn. Codex: `developerInstructions` on `thread/start` and `thread/resume`.
- The workspace chat agent's frame moves to the same mechanism. Today it is sent only on a thread's first turn.

## Status

Derived on read, not stored:

- **Draft**: not started.
- **Running**: a turn or a run is in flight, and no run is waiting on a human gate.
- **Needs you**: open, and either a run is waiting on a human gate, or nothing is in flight or pending. The second case covers a budget refusal, a failed turn, and the agent ending its turn with a question: in each case only you can move it forward.
- **Closed**: `closed` is set.

The attention feed lists missions that need you.

## Remove

Everything that existed only to serve reactions, hooks, or the stage board:

- Stages: the `stage` field, `set_stage`, the Review auto-move, the stage picker, and the six lanes.
- Reactions: `missions/react.yaml` and its assets directory, directive parsing, `reaction_flow`, `reaction_events`, `pending_events` batching, the `reaction` roster role and the `Role` type, and the reactions budget.
- Hooks: the `hooks` field, hook matching, the hook and budget YAML editor, and the `Action` type.
- State and holds: the `state` field and single-writer rule, `hold`, the stored `execution` substate and `settle`, and the attention-hold and budget-wait logic added in CR-2026-0122.
- Pause and resume: their routes, CLI, controls, and the `paused` field. With no reactions or holds to release, sending a message or canceling covers what they did.
- Run signals: `run.signal`, `context.mission.signal`, and the checkpoint read that delivers it. No flow writes it.
- Inbox reads for the UI: `GET /missions/{id}/events`, `spark mission events`, and the Events section. The transcript replaces them.
- Roster attribution fields that only served hooks and reactions: `label`, `role`, and `launched_by_event`.
- The tests, guide sections, and CLI help for all of the above.

## UI

- The board becomes one list, grouped Needs you, Running, Drafts, Closed; newest update first within a group; empty groups hidden. Each row shows title, status line, and last update. Closed rows offer Archive. Search and Show archived stay.
- The detail pane is the transcript. The objective is pinned at the top, followed by the conversation, with run launches and run events inline and linked to the Runs view. The reply box is at the bottom. Drafts show the objective and Start in place of the reply box.
- The header shows title and status, plus a menu with Edit, Cancel, Close, Archive, and Budget.

## Verification

- Service tests:
  - legacy records load, and unstarted done/review records are archived;
  - Start creates the conversation and sends the first turn;
  - events arriving during a turn are delivered as one next turn;
  - at most one turn is in flight;
  - over-budget launches are refused;
  - agent close;
  - cancel;
  - recovery delivers missing terminal events;
  - derived status for each group, including a run waiting on a gate;
  - project isolation.
- Adapter tests: the pinned instructions reach Claude Code as `--append-system-prompt-file` and Codex as `developerInstructions` on start and resume, and they are not added to the turn prompt.
- Contract test end to end: start a mission; its agent launches two runs; both events arrive as turns; the agent closes the mission; the transcript contains the launches, events, and close.
- CLI tests: `spark convo run-request` launches directly; `spark mission close`; removed commands are gone.
- Frontend tests: grouped list and ordering, transcript rendering, reply posts a message, pinned objective, Start on drafts, header menu actions, Archive on closed rows.
- Browser smoke at 1440 and 390 in light and dark, with drafts, running, needs-you, and closed missions.
- `just test`.

## Out of scope

- Deterministic follow-up rules (hooks). Add them if routine steps cost too much agent time.
- Delivering trigger events into missions (slice two of CR-2026-0121).
- Answering a human gate inside a mission's run from the transcript.
