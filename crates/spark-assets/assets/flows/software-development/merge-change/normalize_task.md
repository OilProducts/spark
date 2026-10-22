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
  response: status_envelope
routes:
- to: prepare_integration
  condition: outcome=success
---

Normalize the integration request. context.request.source_ref names the branch or commit to integrate; fail when it is missing or does not resolve in this repository. Resolve the validation command: use context.request.validation_command when provided, otherwise discover the truthful project validation command from the repository (build files, CI configuration, contributor docs) without running it. Do not modify any files. Respond with ONLY a JSON status envelope whose context_updates set context.task.source_ref and context.task.validation_command.
