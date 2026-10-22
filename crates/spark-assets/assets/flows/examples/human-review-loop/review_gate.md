---
type: human_gate
title: Human Review
runtime:
  error_policy: continue
execution:
  reasoning_effort: xhigh
routes:
- to: revise
  label: Request Fixes
  extensions:
    loop_restart: false
- to: ship
  label: Approve
  extensions:
    loop_restart: false
---

Review the proposed change. Approve it to move forward, or request fixes before shipping.
