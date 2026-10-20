---
type: parallel
title: Prove And Refute
config:
  join_policy: wait_all
  max_parallel: 2
routes:
- to: prove
- to: refute
- to: join
  weight: 1
---
