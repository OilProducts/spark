---
type: agent_task
title: Plan
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
- to: implement
  extensions:
    loop_restart: false
---

Inspect the repository and produce a concrete implementation plan for the stated goal. When present, use context.request.summary, context.request.target_paths, and context.request.acceptance_criteria to scope the plan. If the carryover context includes prior implementation failure or review feedback, use it to refine the next attempt instead of restating the goal.
