---
type: parallel
title: Audit Lenses
config:
  branches:
  - lens_correctness
  - lens_minimality
  - lens_safety
  - lens_test_gaps
  join: fan_in
  max_parallel: 4
  tolerate_branch_execution_errors: true
---
