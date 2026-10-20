---
type: agent_task
title: Finalize
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: exit
  extensions:
    loop_restart: false
---

Finalize the approved change and summarize what is ready to land.
