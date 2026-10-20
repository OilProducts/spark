---
type: parallel
title: Propose Approaches
config:
  join_policy: wait_all
  max_parallel: 3
routes:
- to: propose_compat
- to: propose_minimal
- to: propose_refactor
---
