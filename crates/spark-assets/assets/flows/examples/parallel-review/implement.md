---
type: agent_task
title: implement
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: approval
  extensions:
    loop_restart: false
---

Implement the planned changes
