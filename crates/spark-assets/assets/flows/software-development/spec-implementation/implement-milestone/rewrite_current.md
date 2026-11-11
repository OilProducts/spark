---
type: agent_task
title: Rewrite Current
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
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate_item_plan
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: rewrite_current
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read the milestone state in <internal.project_work_dir>/spec-implementation/current/milestones/<current milestone id>/state.json and milestone_dir/rewrite-items.json. Treat context.item.id as the authoritative live binding for the item being rewritten. Mark only that matching item status as superseded, insert the replacement items immediately after it in the items array, set each replacement item status to pending and attempts to 0 unless already provided, clear current_item_id, and keep the milestone state valid JSON. Use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null so the superseded live binding is removed before the replacement queue is dispatched. If context.item.id is missing or cannot be matched truthfully in the durable ledger, identify the item to supersede from rewrite-items.json and the ledger's in_progress item; if none can be identified, append the replacement items to the end of the queue without superseding any item. Choose `done` only after the milestone state is updated and the live active-item binding is cleared. Respond with ONLY a JSON decision envelope.
