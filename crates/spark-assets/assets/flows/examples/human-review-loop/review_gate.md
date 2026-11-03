---
type: human_gate
title: Human Review
execution:
  reasoning_effort: xhigh
routes:
  approve:
    to: ship
    description: The proposal is approved for shipping.
    label: Approve
  request_fixes:
    to: revise
    description: The proposal needs revision; record the reviewer’s feedback.
    label: Request Fixes
  stop:
    to: failed
    description: Stop this workflow without approving the proposal.
    label: Stop Run
---

Review the proposed change. Approve it to move forward, request fixes before shipping, or stop the run without approving it.
