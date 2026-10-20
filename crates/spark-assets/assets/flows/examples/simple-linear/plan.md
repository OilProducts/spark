---
type: agent_task
title: Plan
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

Inspect the repository and plan the targeted improvement needed to satisfy the stated goal.
