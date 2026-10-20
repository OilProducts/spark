---
type: agent_task
title: plan
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: implement
  extensions:
    loop_restart: false
---

Plan the implementation
