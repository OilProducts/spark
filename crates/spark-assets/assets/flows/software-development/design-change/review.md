---
type: parallel
title: Review Design
config:
  branches:
  - codebase_check
  - critic
  - naive_reader
  - readiness
  - zoom_out
  join: fan_in
  max_parallel: 5
  tolerate_branch_execution_errors: true
runtime:
  max_visits: 3
---
