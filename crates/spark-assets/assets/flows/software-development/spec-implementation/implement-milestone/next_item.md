---
type: agent_task
title: Next Item
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
  label: Blocked
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: final_milestone_audit
  label: Audit
  condition: outcome=success && preferred_label=Audit
  extensions:
    loop_restart: false
- to: plan_current
  label: Work
  condition: outcome=success && preferred_label=Work
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Repair Queue
  condition: outcome=fail && preferred_label=Replan
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Repair Queue Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
- to: final_milestone_audit
  label: Audit Partial
  condition: outcome=partial_success && preferred_label=Audit
  weight: 1
  extensions:
    loop_restart: false
- to: plan_current
  label: Work Partial
  condition: outcome=partial_success && preferred_label=Work
  weight: 1
  extensions:
    loop_restart: false
---

Read the current milestone state in .spark/spec-implementation/current/milestones/<current milestone id>/state.json and choose the next ready work item. If current_item_id names the sole in_progress item and context.item.id matches it, treat this as a controlled child restart: preserve state.json exactly, do not increment attempts, return outcome success with preferred_label Work, and emit context_updates for context.item.* from that item while clearing the review feedback and validation scratch keys listed below. A ready item is pending and all of its depends_on items are already done. If a ready item exists, store its id and previous attempts as selected_id and previous_attempts before editing. Update that same state.json file before responding: modify only the item whose id equals selected_id, mark that item status=in_progress, set its attempts to previous_attempts + 1, set current_item_id to selected_id, preserve the rest of the queue, and keep the file valid JSON. Do not assume context_updates, routing, or any external orchestrator will update milestone_dir/state.json; the durable ledger write is this node's responsibility and must happen before the final status envelope. After writing, reread state.json from disk and verify all Work-path invariants before responding: current_item_id equals selected_id; exactly one item has status=in_progress; that sole in_progress item's id equals selected_id; that item's attempts equal previous_attempts + 1; no other item changed status; and the context.item.id you will emit equals selected_id. If any invariant fails, repair state.json and reread it again before responding. If the invariants still cannot be made true, return outcome fail with preferred_label Replan rather than emitting Work, so Validate Item Plan can repair the queue. After the file is written and reread with all Work-path invariants true, respond with ONLY a JSON status envelope with outcome success, preferred_label Work, and context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, plus cleared context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code. Return outcome success with preferred_label Work only after state.json on disk agrees with the emitted context.item.* binding and the selected item is the sole in_progress item. If no pending or in_progress items remain, clear current_item_id in state.json before responding and return outcome success with preferred_label Audit. On the Audit path, include context_updates that set the active item keys, review feedback keys, and validation scratch keys to JSON null so the live binding is fully cleared before milestone audit. Return outcome success with preferred_label Audit only after rereading state.json from disk and confirming current_item_id=null and no item is in_progress. If pending items remain but none are ready, keep the milestone state valid and return outcome fail with preferred_label Replan so Validate Item Plan can restructure the stuck items.
