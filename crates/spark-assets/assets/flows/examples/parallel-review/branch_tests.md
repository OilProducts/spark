---
type: agent_task
title: branch_tests
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

Review test updates
