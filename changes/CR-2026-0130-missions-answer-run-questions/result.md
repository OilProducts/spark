---
id: CR-2026-0130-missions-answer-run-questions
title: Missions handle their runs' questions
status: completed
type: feature
changelog: public
---

## Summary

Missions receive actionable questions from launched runs and all descendants, resolve ownership through the root run, and deduplicate inbox delivery by question ID. Answers use the existing API and restore the root roster from waiting to running. Mission agents answer from settled evidence or ask the user and relay the reply.

## Validation

- `just test`: passed (exit 0): formatting, all Rust suites, 629 frontend tests across 83 files, and the production frontend build. Log: `/tmp/cr130-just-test.log`.
- Mission contracts verify grandchild question delivery and turn triggering, every required question field and option description in the prompt, duplicate suppression across child/root publishes, Running during question handling, Needs you and attention-feed inclusion after an unanswered turn, user reply delivery and relay instructions, external API answers clearing roster waits, and continuation after completion.
- A live subflow contract answers the owning child's human gate through the existing API and verifies child resumption, root completion, roster recovery, and the mission's completion turn.
- CLI contracts verify descendant question output, option and text answer bodies addressed to the owning child, and rejection of missing or mutually supplied answer flags.
- `git diff --check`: passed. The frontend emitted non-failing React act warnings and build bundle-size advisories.

## Shipped Changes

- Root-based mission ownership lookup and question reconciliation on the existing observed run publication path; one `run.question` event per question ID with owning run, root run, node, prompt, and complete options.
- Question-derived roster waits, clearing when answers arrive through any route; `run.waiting` retained for recovery decisions.
- Active agent turns take precedence over waiting status; unanswered questions require user attention after the turn ends.
- `spark run questions --run <id>` and `spark run answer --run <owning run id> --question <id> (--option <value> | --text <text>)` over existing routes.
- Mission instructions and operations guidance explain evidence-based answers, user escalation, ending the turn, and relaying replies without guessing. No transcript answer buttons were added.
