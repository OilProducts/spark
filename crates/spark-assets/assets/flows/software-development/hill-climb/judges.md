---
type: parallel
title: Judges
config:
  branches:
  - opus
  - sol
  join: judges_join
  max_parallel: 2
  tolerate_branch_execution_errors: true
runtime:
  max_visits: 64
---
