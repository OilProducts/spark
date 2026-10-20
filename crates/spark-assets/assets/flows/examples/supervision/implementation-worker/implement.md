---
type: agent_task
title: Implement
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.request.summary
  - context.request.target_paths
  - context.request.acceptance_criteria
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: review
  label: Review
  condition: outcome=success
  extensions:
    loop_restart: false
---

Attempt the repository change needed to satisfy the stated goal, then validate the result before handing it off for review. When present, honor context.request.summary, context.request.target_paths, and context.request.acceptance_criteria. If the carryover context includes review feedback or required changes, address them directly in this pass.
