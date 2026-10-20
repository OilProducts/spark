---
type: agent_task
title: branch_docs
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: fan_in
  extensions:
    loop_restart: false
---

Review documentation updates
