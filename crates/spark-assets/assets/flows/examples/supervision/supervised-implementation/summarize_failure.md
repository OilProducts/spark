---
type: agent_task
title: Summarize Failure
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.stack.child.status
  - context.stack.child.outcome
  - context.stack.child.outcome_reason_code
  - context.stack.child.outcome_reason_message
  - context.stack.child.active_stage
  - context.stack.child.completed_nodes
  - context.stack.child.route_trace
  - context.stack.child.failure_reason
  - context.stack.child.retry_count
  - context.stack.child.retry_counts
  - context.stack.child.artifact_count
  - context.stack.child.event_count
  - context.stack.child.checkpoint_timestamp
  - context.stack.child.latest_event_at
  - context.stack.child.started_at
  - context.stack.child.ended_at
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: done
  extensions:
    loop_restart: false
---

Summarize why the child workflow failed, what it attempted, and what a human should inspect next.
