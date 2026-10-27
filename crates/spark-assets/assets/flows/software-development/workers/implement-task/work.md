---
type: agent_task
title: Implement
runtime:
  session: implementer
contracts:
  reads_context:
  - context.task.objective
  - context.task.artifact_path
  - context.task.validation_command
  - context.review.summary
  - context.review.required_changes
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

Implement the task described by context.task.objective in the current working directory, which is an isolated git worktree on a dedicated branch. Inspect the repository, make the smallest coherent implementation, and test and review observable behavior; context.task.validation_command names the project validation gate. If context.review.required_changes is present, it is mandatory feedback from the gatekeeper evaluation of your previous attempt; address it directly before finishing. Do not write a change-request result file; the run records its own result. Respect repository ignore rules; do not force-add ignored change requests or evidence. After testing and self-review, commit the task changes on the existing worktree branch with a descriptive message. On subsequent implementation iterations, create a new commit for the fixes; do not amend or rewrite earlier commits. If there are no changes, do not create an empty commit. Do not push, merge, create branches or worktrees, or clean anything up; the parent flow evaluates, validates, finalizes, and releases the workspace. Respond with ONLY a JSON status envelope.
