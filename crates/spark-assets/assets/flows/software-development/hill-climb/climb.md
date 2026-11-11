---
type: agent_task
title: Climb
runtime:
  session: climber
  workdir_from: context.workspace.path
  max_visits: 64
contracts:
  reads_context:
  - context.climb.objective
  - context.climb.rubric
  - context.climb.target_score
  - context.climb.evidence_command
  - context.climb.findings
  - context.climb.history
  - context.climb.capture_failed
  - context.climb.capture_log
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: Raising the scores further needs input or authority you do not have; record the blocker in notes.
  ready:
    to: capture
    description: This round's improvements are committed and ready for the council to score.
  unfinished:
    to: climb
    description: This round's work remains and can be continued in the same session.
---

Improve the work in this worktree, which is on its own branch, toward context.climb.objective, so that an independent council scores every category of context.climb.rubric at or above context.climb.target_score. context.climb.findings holds each member's findings from the latest round, and context.climb.history holds every round's scores with the commit they scored, so you can see which changes moved them. When context.climb.capture_failed is true, context.climb.evidence_command failed on your last commit and its output is at context.climb.capture_log; make it succeed again first.

The council scores the evidence, so raise scores only by improving the work itself. Never change what the evidence command captures, the references, or tests and checks to move a score. A finding that would break the objective's constraints is not worth its score.

Commit your changes with a message saying what changed; do not amend or rewrite earlier commits, and do not push or merge. Choose `ready` once this round's improvements are committed. Respond with ONLY a JSON decision envelope.
