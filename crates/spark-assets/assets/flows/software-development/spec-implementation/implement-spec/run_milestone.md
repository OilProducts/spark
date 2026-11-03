---
type: subflow
title: Run Milestone Worker
config:
  flow_ref: implement-milestone
  results:
    blocked: child_blocked
    completed: child_completed
runtime: {}
manager:
  poll_interval: 0ms
  max_cycles: 1
execution:
  reasoning_effort: xhigh
routes:
  child_blocked:
    to: audit_milestone
    description: The milestone child reached blocked; audit its evidence to choose continuation, rewriting or an honest external block.
  child_completed:
    to: audit_milestone
    description: The milestone child reached completed; independently audit its artifacts and deliverability.
---
