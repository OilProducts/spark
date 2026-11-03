---
type: agent_task
title: Implement
contracts:
  reads_context:
  - context.request.summary
  - context.request.target_paths
  - context.request.acceptance_criteria
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: review
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: implement
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Attempt the repository change needed to satisfy the stated goal, then validate the result before handing it off for review. When present, honor context.request.summary, context.request.target_paths, and context.request.acceptance_criteria. If the carryover context includes review feedback or required changes, address them directly in this pass. Respond with ONLY a JSON decision envelope.
