---
type: agent_task
title: Mark Current Blocked
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - internal.project_work_dir
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
    to: next_item
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: mark_current_blocked
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read the current milestone state in <internal.project_work_dir>/spec-implementation/current/milestones/<current milestone id>/state.json and <internal.project_work_dir>/spec-implementation/current/block-reason.md. Treat context.item.id as the authoritative live binding for the item being blocked. Mark only that matching item status as blocked, store a concise reason in its notes field, clear current_item_id, preserve the rest of the queue, keep the milestone state valid JSON. Use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null so the live binding is removed after the block is recorded. If context.item.id is missing, the matching item is absent, or the durable ledger points at a different item, identify the item from block-reason.md and the ledger's in_progress item and mark that item; if no item can be identified, leave every item status unchanged, clear current_item_id, and still clear the live binding so the queue can continue. Choose `done` only after the milestone state is updated and the live active-item binding is cleared. Respond with ONLY a JSON decision envelope.
