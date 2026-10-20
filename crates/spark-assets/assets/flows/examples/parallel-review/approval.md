---
type: human_gate
title: approval
runtime:
  error_policy: continue
execution:
  reasoning_effort: xhigh
routes:
- to: exit
  label: Cancel
  condition: preferred_label=Cancel
  extensions:
    loop_restart: false
- to: parallel_work
  label: Proceed
  condition: preferred_label=Proceed
  extensions:
    loop_restart: false
---

Approve moving to parallel review
