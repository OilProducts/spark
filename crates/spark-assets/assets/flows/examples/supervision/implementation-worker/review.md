---
type: agent_task
title: Review
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.request.summary
  - context.request.target_paths
  - context.request.acceptance_criteria
  writes_context:
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: done
  label: Ready
  condition: outcome=success
  extensions:
    loop_restart: false
- to: implement
  label: Fix
  condition: outcome=fail
  extensions:
    loop_restart: false
---

Review the implementation for correctness, validation coverage, and remaining issues. When present, judge it against context.request.summary, context.request.target_paths, and context.request.acceptance_criteria. Respond with ONLY a JSON status envelope. Return outcome success only when the change is ready. Otherwise return outcome fail with failure_reason and context_updates for context.review.summary, context.review.required_changes, and context.review.blockers.
