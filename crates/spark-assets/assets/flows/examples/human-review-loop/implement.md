---
type: agent_task
title: Prepare Change
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: review_gate
  extensions:
    loop_restart: false
---

Inspect the repository, prepare the proposed change for the stated goal, and summarize what is ready for review.
