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
- to: plan
  label: Replan
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: review
  label: Review
  condition: outcome=success
  extensions:
    loop_restart: false
---

Make the planned change in the repository and validate that it addresses the stated goal. When present, honor context.request.summary, context.request.target_paths, and context.request.acceptance_criteria. If the carryover context includes review feedback or required changes, address them directly in this pass.
