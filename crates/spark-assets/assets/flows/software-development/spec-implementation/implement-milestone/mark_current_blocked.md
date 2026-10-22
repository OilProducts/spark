---
type: agent_task
title: Mark Current Blocked
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
- to: next_item
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked State Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read the current milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json and .spark/spec-implementation/current/block-reason.md. Treat context.item.id as the authoritative live binding for the item being blocked. Mark only that matching item status as blocked, store a concise reason in its notes field, clear current_item_id, preserve the rest of the queue, keep the milestone state valid JSON, and respond with ONLY a JSON status envelope. Use context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code by setting each to JSON null so the live binding is removed after the block is recorded. If context.item.id is missing, the matching item is absent, or the durable ledger points at a different item, identify the item from block-reason.md and the ledger's in_progress item and mark that item; if no item can be identified, leave every item status unchanged, clear current_item_id, and still clear the live binding so the queue can continue. Return outcome success only after the milestone state is updated and the live active-item binding is cleared.
