---
type: agent_task
title: Mark Current Done
runtime: {}
contracts:
  reads_context:
  - context.item.id
  writes_context:
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.item.decision_ids
  - context.item.acceptance_criteria
  - context.item.constraints
  - context.item.target_paths
  - context.item.depends_on
  - context.item.attempts
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  - context.validation.item_id
  - context.validation.status
  - context.validation.summary
  - missing_prerequisites
  - validation_message
  - validation_exit_code
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: commit_current
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  revalidate:
    to: prepare_validation
    description: The active item lacks current passing validation evidence; preserve its incomplete state and record precisely what must be rerun.
  unfinished:
    to: mark_current_done
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read the current milestone state in .spark/state/spec-implementation/current/milestones/<current milestone id>/state.json and .spark/state/spec-implementation/current/validation-result.json. Treat context.item.id as the only item that may be completed in this node. Require state.json.current_item_id and .spark/state/spec-implementation/current/validation-result.json.item_id to both equal context.item.id, require the matching state item to exist exactly once with status=in_progress, and require validation status=pass with all required suites passing. Mark only that matching item status as completed, clear current_item_id, preserve the rest of the queue, and keep valid JSON. Do not modify any other item status in this node. Use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null so the live binding is removed after completion. If the ledger's identity fields disagree with context.item.id, reconcile them to context.item.id from the evidence first. If validation evidence for context.item.id is missing or non-passing, do not mark the item done: choose `revalidate` without mutating queue state. Choose `done` only after the milestone state is updated and the live active-item binding is cleared. Respond with ONLY a JSON decision envelope.
