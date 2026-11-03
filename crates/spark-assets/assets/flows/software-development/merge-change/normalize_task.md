---
type: agent_task
title: Normalize Task
contracts:
  reads_context:
  - context.request.source_ref
  - context.request.validation_command
  writes_context:
  - context.task.source_ref
  - context.task.validation_command
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: prepare_integration
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: normalize_task
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Normalize the integration request. context.request.source_ref names the branch or commit to integrate; choose `blocked` when it is missing or does not resolve in this repository. Resolve the validation command: use context.request.validation_command when provided, otherwise discover the truthful project validation command from the repository (build files, CI configuration, contributor docs) without running it. Do not modify any files. Choose `done` once context_updates set context.task.source_ref and context.task.validation_command. Respond with ONLY a JSON decision envelope.
