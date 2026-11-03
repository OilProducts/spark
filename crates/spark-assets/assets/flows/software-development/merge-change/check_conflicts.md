---
type: switch
title: Check Conflicts
config:
  cases:
    '0': clean
  on: context.integration.conflict_count
  otherwise: conflicts
contracts:
  reads_context:
  - context.integration.conflict_count
routes:
  clean:
    to: finish_merge
    description: Integration preparation completed and the staged merge has zero unresolved conflicts.
  conflicts:
    to: resolve_conflicts
    description: Integration preparation completed and the staged merge contains unresolved conflicts.
---
