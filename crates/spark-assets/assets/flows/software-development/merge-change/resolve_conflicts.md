---
type: subflow
title: Resolve Conflicts
config:
  flow_ref: workers/resolve-merge-conflicts
  input_map:
    context.integration.source_commit: context.integration.source_commit
    context.integration.target_commit: context.integration.target_commit
manager:
  child_workdir_from: context.integration.path
routes:
- to: validate
  condition: outcome=success
---
