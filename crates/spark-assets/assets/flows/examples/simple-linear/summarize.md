---
type: agent_task
title: Summarize
runtime:
  error_policy: continue
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: done
  extensions:
    loop_restart: false
---

Summarize what changed, what was validated, and any remaining follow-up for the stated goal.
