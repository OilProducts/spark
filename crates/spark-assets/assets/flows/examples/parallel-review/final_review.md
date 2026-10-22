---
type: agent_task
title: final_review
runtime:
  error_policy: continue
contracts:
  reads_context:
  - parallel.results
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: exit
  extensions:
    loop_restart: false
---

Finalize and summarize outputs
