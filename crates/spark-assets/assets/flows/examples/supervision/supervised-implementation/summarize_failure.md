---
type: agent_task
title: Summarize Failure
contracts:
  reads_context:
  - stack.child.status
  - stack.child.outcome
  - stack.child.outcome_reason_code
  - stack.child.outcome_reason_message
  - stack.child.active_stage
  - stack.child.completed_nodes
  - stack.child.route_trace
  - stack.child.failure_reason
  - stack.child.retry_count
  - stack.child.retry_counts
  - stack.child.artifact_count
  - stack.child.event_count
  - stack.child.checkpoint_timestamp
  - stack.child.latest_event_at
  - stack.child.started_at
  - stack.child.ended_at
  response: decision_envelope
execution:
  llm_model: gpt-6.2-sol
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
