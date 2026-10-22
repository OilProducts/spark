---
type: agent_task
title: Implement
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: summarize
  extensions:
    loop_restart: false
---

Carry out the planned improvement in the repository and validate the result before moving on.
