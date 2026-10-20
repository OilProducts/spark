---
type: parallel
title: parallel_work
config:
  join_policy: wait_all
  max_parallel: 2
runtime:
  error_policy: continue
execution:
  reasoning_effort: xhigh
routes:
- to: branch_docs
  extensions:
    loop_restart: false
- to: branch_tests
  extensions:
    loop_restart: false
---
