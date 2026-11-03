---
type: agent_task
title: Resolve Conflicts
runtime:
  workdir_from: context.integration.path
contracts:
  reads_context:
  - context.integration.source_commit
  - context.integration.target_commit
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: resolve_conflicts
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

The current working directory is an integration worktree with an in-progress merge (--no-ff --no-commit). If git reports no conflicted paths, choose `done` without changing anything. Otherwise resolve only conflicts whose behavior-preserving resolution is unambiguous and stage the resolved files, then choose `done`; when any conflict requires judgment about intended behavior, choose `blocked`, name those conflicts in notes, and leave the worktree inspectable. Do not commit, validate, move branches, or clean up; later steps perform those operations. Respond with ONLY a JSON decision envelope.
