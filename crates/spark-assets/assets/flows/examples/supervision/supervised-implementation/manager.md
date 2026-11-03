---
type: subflow
title: Run Child Worker
config:
  flow_ref: implementation-worker
  results:
    blocked: child_blocked
    completed: child_completed
manager:
  poll_interval: 5s
  max_cycles: 6
  actions:
  - observe
  - wait
execution:
  reasoning_effort: xhigh
routes:
  child_blocked:
    to: summarize_failure
    description: The child reached its declared blocked failure result; summarize its preserved blocker evidence.
  child_completed:
    to: summarize_success
    description: The child reached its declared completed success result; summarize that result.
---
