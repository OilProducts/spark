---
type: parallel
title: Review Design
config:
  join_policy: wait_all
  max_parallel: 5
runtime:
  max_visits: 2
routes:
- to: codebase_check
- to: critic
- to: naive_reader
- to: readiness
- to: zoom_out
- to: fan_in
  weight: 1
---
