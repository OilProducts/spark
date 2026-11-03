---
type: agent_task
title: Summarize Failure
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
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: failed
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: summarize_failure
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Summarize why the child workflow failed, what it attempted, and what a human should inspect next. Respond with ONLY a JSON decision envelope.
