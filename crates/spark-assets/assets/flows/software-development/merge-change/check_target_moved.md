---
type: switch
title: Check Target Moved
config:
  cases:
    'false': tested
  on: context.integration.target_moved
  otherwise: untested
contracts:
  reads_context:
  - context.integration.target_moved
routes:
  tested:
    to: finish_merge
    description: The target commit is the merge base, so the merged tree is the source tree already tested.
  untested:
    to: validate
    description: The target moved past the merge base, so the merged tree has never been tested.
---
