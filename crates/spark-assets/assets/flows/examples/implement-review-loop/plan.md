---
type: agent_task
title: Plan
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
    to: implement
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: plan
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Inspect the repository and produce a concrete implementation plan for the stated goal. When present, use context.request.summary, context.request.target_paths, and context.request.acceptance_criteria to scope the plan. If the carryover context includes prior implementation failure or review feedback, use it to refine the next attempt instead of restating the goal. Respond with ONLY a JSON decision envelope.
