# Missions handle their runs' questions

## Summary

When a run a mission launched, or any of that run's child runs, stops on a question (a human gate or an agent clarification), the mission's agent receives the question as a turn and can answer it: itself when the objective, playbook, and evidence settle it, or by asking the user and relaying their reply. The mission shows Needs you only when the question is waiting on the user.

## Why

On 2026-09-27 a mission's implementation run stopped for 52 minutes on a question raised inside the `implement` subflow's child run. The mission never heard about it and kept showing Running:

- Mission delivery (`deliver_run_events` in `crates/spark-workspace/src/missions.rs`) only follows runs on the mission's roster. A child run's waiting state never reaches the mission, and its root run stays `running`.
- A delivered `run.waiting` says only "is waiting on a human gate", with no question, and the agent has no command to answer one.
- A roster entry stays `waiting` until the run ends (the `ponytail:` note in `missions.rs`), so an answered run still reads as waiting.

## What exists

- `GET /attractor/pipelines/{id}/questions` (`list_pipeline_questions` in `crates/attractor-api/src/lib.rs`) returns open questions for a run and all its descendants, each tagged with the run that owns it.
- `POST /attractor/pipelines/{id}/questions/{question_id}/answer` (`answer_pipeline_question`) answers a question on the run that owns it.
- Child runs record their parent and root run; the root run's launch context carries `context.spark_mission`.

## Delivery

- When any run in a mission-owned run tree posts a question, append one `run.question` event to the mission's inbox, keyed by question id so repeated publishes deliver once. Find the mission through the run's root run.
- The event carries the owning run id, the root run id, node id, prompt, options (label, value, description), and question id. The turn rendering shows these so the agent can act without another lookup.
- When the question is answered (by the agent, the user in the Runs view, or anyone else), the owning run resumes and the roster entry for its root run returns from `waiting` to running. Replace the `ponytail:` shortcut.
- Keep `run.waiting` for waits that are not questions, if any remain; otherwise fold it into `run.question`.

## Answering

Add to the `spark` CLI, over the existing routes:

- `spark run questions --run <id>`: open questions for the run and its descendants.
- `spark run answer --run <owning run id> --question <question id> (--option <value> | --text <text>)`.

Add to the mission frame (`mission_frame` in `missions.rs`): when a run asks a question, answer it with `spark run answer` when the objective, the playbook, and the evidence settle it. When the decision is the user's, or the question is unclear, ask the user and end the turn, then relay their reply as the answer. Never guess an answer.

## Status

Derived as today, with questions:

- **Running**: a turn is in flight (including the turn handling a question), or runs are in flight with no open question.
- **Needs you**: open, and nothing can proceed without the user. This includes an open run question after the agent's turn has ended without answering it.

The attention feed lists missions that need you.

## Verification

- A question raised in a mission run's child run reaches the mission as one `run.question` event with the prompt, options, and owning run, and triggers a turn.
- The agent answers through `spark run answer`; the child run resumes, the roster entry leaves `waiting`, and the mission returns to Running, then proceeds when the run ends.
- An agent that ends its turn without answering leaves the mission in Needs you; the user's reply arrives as a turn and the agent relays it.
- A question answered outside the mission (Runs view) also resumes the run and clears the mission's waiting state.
- Duplicate publishes do not duplicate events.
- CLI tests for `run questions` and `run answer` (option and text), including a child-run question addressed by its owning run.
- `just test`.

## Out of scope

- Answer buttons in the mission transcript. The user replies by message and the agent relays it.

## Hand-off

Write `changes/CR-2026-0130-missions-answer-run-questions/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
