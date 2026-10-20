---
type: subflow
title: Run Milestone Worker
config:
  flow_ref: implement-milestone
runtime: {}
manager:
  poll_interval: 0ms
  max_cycles: 1
execution:
  reasoning_effort: xhigh
routes:
- to: audit_milestone
  label: Audit Failure
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: audit_milestone
  label: Audit Success
  condition: outcome=success
  extensions:
    loop_restart: false
---
