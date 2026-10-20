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
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked State
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: commit_current
- to: prepare_validation
  label: Revalidate
  condition: outcome=fail && preferred_label=Revalidate
  extensions:
    loop_restart: false
---

Read the current milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json and .spark/spec-implementation/current/validation-result.json. Treat context.item.id as the only item that may be completed in this node. Require state.json.current_item_id and .spark/spec-implementation/current/validation-result.json.item_id to both equal context.item.id, require the matching state item to exist exactly once with status=in_progress, and require validation status=pass with all required suites passing. Mark only that matching item status as completed, clear current_item_id, preserve the rest of the queue, keep valid JSON, and respond with ONLY a JSON status envelope. Do not modify any other item status in this node. Use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null so the live binding is removed after completion. If the ledger's identity fields disagree with context.item.id, reconcile them to context.item.id from the evidence first. If validation evidence for context.item.id is missing or non-passing, do not mark the item done: return outcome fail with preferred_label Revalidate without mutating queue state. Return outcome success only after the milestone state is updated and the live active-item binding is cleared.
