---
type: parallel
title: parallel_work
config:
  branches:
  - branch_docs
  - branch_tests
  join: fan_in
  max_parallel: 2
  tolerate_branch_execution_errors: true
execution:
  reasoning_effort: xhigh
---
