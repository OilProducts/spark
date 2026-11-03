---
type: agent_task
title: Review
contracts:
  reads_context:
  - context.request.summary
  - context.request.target_paths
  - context.request.acceptance_criteria
  writes_context:
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
  fix:
    to: implement
    description: The completed review or validation assessment found concrete defects requiring another implementation pass on the same item; record actionable findings.
  ready:
    to: done
    description: The review is complete and no blocking issue remains under this node’s stated acceptance criteria; record all required review evidence.
  unfinished:
    to: review
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Review the implementation for correctness, validation coverage, and remaining issues. When present, judge it against context.request.summary, context.request.target_paths, and context.request.acceptance_criteria. Choose `ready` only when the change is ready. Otherwise choose `fix`, explaining why in notes and setting context_updates for context.review.summary, context.review.required_changes, and context.review.blockers. Choose `blocked` only when you cannot perform the review at all. Respond with ONLY a JSON decision envelope.
