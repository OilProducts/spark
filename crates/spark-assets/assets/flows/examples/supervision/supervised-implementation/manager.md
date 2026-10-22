---
type: subflow
title: Run Child Worker
config:
  flow_ref: implementation-worker
runtime:
  error_policy: continue
manager:
  poll_interval: 5s
  max_cycles: 6
  actions:
  - observe
  - wait
execution:
  reasoning_effort: xhigh
routes:
- to: summarize_failure
  label: Report Failure
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: summarize_success
  label: Summarize
  condition: outcome=success
  extensions:
    loop_restart: false
---
