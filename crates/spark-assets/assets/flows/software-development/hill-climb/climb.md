---
type: agent_task
title: Climb
runtime:
  workdir_from: context.workspace.path
  max_visits: 64
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_score
  - context.climb.findings
  - context.climb.history
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: Raising the scores further needs input or authority you do not have; record the blocker in notes.
  ready:
    to: sync
    description: This round's improvements are committed and ready for the judges.
  unfinished:
    to: climb
    description: This round's work remains; a fresh climber continues it from the worktree.
---

Improve the work in this worktree, which is on its own branch, toward context.request.objective, so that every judge scores it at or above context.request.target_score out of 10. The judges observe the work fresh each round and never see the target or earlier scores. context.climb.findings holds each judge's score and findings from the latest round, and context.climb.history holds every round's scores with the commit they scored, so you can see which changes moved them.

Raise scores only by improving the work for its users; a change aimed at the judges is not an improvement. A finding that would break the objective's constraints is not worth its score. Existing tests describe the work as it was, not what it must become: when an improvement breaks a test, update or remove the test to match instead of holding the improvement back, unless the objective says the test must hold.

Commit your changes with a message saying what changed; do not amend or rewrite earlier commits, and do not push or merge. Choose `ready` once this round's improvements are committed. Respond with ONLY a JSON decision envelope.
