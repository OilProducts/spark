---
type: agent_task
title: Implement
role: author
runtime:
  session: implementer
  workdir_from: context.workspace.path
  max_visits: 5
contracts:
  reads_context:
  - context.task.objective
  - context.task.artifact_path
  - context.task.validation_command
  - context.review.summary
  - context.review.required_changes
  writes_context:
  - context.review.contested
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready_for_review:
    to: evaluate
    description: The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats.
  unfinished:
    to: implement
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Implement the task described by context.task.objective in the current working directory, which is an isolated git worktree on a dedicated branch. Inspect the repository, make the smallest coherent implementation, and test and review observable behavior; context.task.validation_command names the project validation gate. If context.review.required_changes is present, it lists findings from the gatekeeper evaluation of your previous attempt; resolve each one as your role describes before finishing. Do not write a change-request result file; the run records its own result. Respect repository ignore rules; do not force-add ignored change requests or evidence. After testing and self-review, commit the task changes on the existing worktree branch with a descriptive message. On subsequent implementation iterations, create a new commit for the fixes; do not amend or rewrite earlier commits. If there are no changes, do not create an empty commit. Do not push, merge, create branches or worktrees, or clean anything up; later steps evaluate, validate, finalize, and release the workspace. Choose `ready_for_review` once this implementation pass is committed and ready for the independent evaluation, recording any caveats in notes; choose `unfinished` only when required implementation work remains that you can continue in this session. Respond with ONLY a JSON decision envelope.
