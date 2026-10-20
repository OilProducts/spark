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
  response: status_envelope
routes:
- to: prepare_workspace
  condition: outcome=success
---

Normalize the launch request into one bounded task. Treat exactly one of context.request.objective or context.request.artifact_path as the task source; an approved change-request document such as changes/<CR-id>/request.md is a valid artifact source whose requirements are the contract; fail when both or neither are provided, and fail on absolute or repository-escaping artifact paths. Set context.task.artifact_path to the exact repository-relative artifact path for artifact requests, or an empty string for inline objectives; workspace preparation carries that file into the isolated checkout. Fold optional target_paths, constraints, and acceptance_criteria into a single decision-complete task statement. Resolve the validation command: use context.request.validation_command when provided, otherwise discover the truthful project validation command from the repository (build files, CI configuration, contributor docs) without running it. Resolve the base ref: use context.request.base_ref when provided, otherwise HEAD. Do not modify any files. Respond with ONLY a JSON status envelope whose context_updates set context.task.objective, context.task.artifact_path, context.task.validation_command, and context.task.base_ref.
