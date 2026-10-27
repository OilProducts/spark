---
type: subflow
title: Implement
config:
  flow_ref: workers/implement-task
  input_map:
    context.review.required_changes: context.review.required_changes
    context.review.summary: context.review.summary
    context.task.artifact_path: context.task.artifact_path
    context.task.objective: context.task.objective
    context.task.validation_command: context.task.validation_command
  output_map:
    context.review.contested: context.review.contested
runtime:
  max_entries: 5
contracts:
  writes_context:
  - context.review.contested
manager:
  child_workdir_from: context.workspace.path
routes:
- to: evaluate
  condition: outcome=success
---
