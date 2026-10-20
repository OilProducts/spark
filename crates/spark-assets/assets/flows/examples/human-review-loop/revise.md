---
type: agent_task
title: Apply Fixes
runtime:
  error_policy: continue
contracts:
  reads_context:
  - human.gate.label
  - human.gate.note
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: review_gate
  extensions:
    loop_restart: false
---

Address the human feedback (the reviewer's note arrives as the human.gate.note context value; when it is empty, re-inspect the proposed change for whatever a reviewer would most plausibly reject), update the proposed change, and return it for review.
