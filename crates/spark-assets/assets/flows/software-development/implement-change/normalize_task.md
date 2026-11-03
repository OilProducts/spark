---
type: agent_task
title: Normalize Task
contracts:
  reads_context:
  - context.request.objective
  - context.request.artifact_path
  - context.request.target_paths
  - context.request.constraints
  - context.request.acceptance_criteria
  - context.request.validation_command
  - context.request.base_ref
  writes_context:
  - context.task.objective
  - context.task.artifact_path
  - context.task.validation_command
  - context.task.base_ref
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: prepare_workspace
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: normalize_task
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Normalize the launch request into one bounded task. Treat exactly one of context.request.objective or context.request.artifact_path as the task source; an approved change-request document such as changes/<CR-id>/request.md is a valid artifact source whose requirements are the contract; choose `blocked` when both or neither are provided, and choose `blocked` on absolute or repository-escaping artifact paths. Set context.task.artifact_path to the exact repository-relative artifact path for artifact requests, or an empty string for inline objectives; workspace preparation carries that file into the isolated checkout. Fold optional target_paths, constraints, and acceptance_criteria into a single decision-complete task statement. Resolve the validation command: use context.request.validation_command when provided, otherwise discover the truthful project validation command from the repository (build files, CI configuration, contributor docs) without running it. Resolve the base ref: use context.request.base_ref when provided, otherwise HEAD. Do not modify any files. Choose `done` once context_updates set context.task.objective, context.task.artifact_path, context.task.validation_command, and context.task.base_ref. Respond with ONLY a JSON decision envelope.
