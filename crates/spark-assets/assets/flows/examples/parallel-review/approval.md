---
type: human_gate
title: approval
execution:
  reasoning_effort: xhigh
routes:
  cancel:
    to: failed
    description: Decline further work and end without approval.
    label: Cancel
  proceed:
    to: parallel_work
    description: Approve running the declared parallel review branches.
    label: Proceed
---

Approve moving to parallel review, or cancel and end without approval.
