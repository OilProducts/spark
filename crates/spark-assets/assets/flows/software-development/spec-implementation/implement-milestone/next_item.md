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
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  audit:
    to: final_milestone_audit
    description: No pending or active work remains in this queue; clear the required live bindings and advance to its independent audit.
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  replan:
    to: validate_item_plan
    description: The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning.
  unfinished:
    to: next_item
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  work:
    to: plan_current
    description: A ready pending item or milestone has been durably dispatched with the required identity, attempt and context invariants verified.
---

Read the current milestone state in .spark/state/spec-implementation/current/milestones/<current milestone id>/state.json and choose the next ready work item. If current_item_id names the sole in_progress item and context.item.id matches it, treat this as a controlled child restart: preserve state.json exactly, do not increment attempts, choose `work`, and emit context_updates for context.item.* from that item while clearing the review feedback and validation scratch keys listed below. A ready item is pending and all of its depends_on items are already done. If a ready item exists, store its id and previous attempts as selected_id and previous_attempts before editing. Update that same state.json file before responding: modify only the item whose id equals selected_id, mark that item status=in_progress, set its attempts to previous_attempts + 1, set current_item_id to selected_id, preserve the rest of the queue, and keep the file valid JSON. Do not assume context_updates, routing, or any external orchestrator will update milestone_dir/state.json; the durable ledger write is this node's responsibility and must happen before the final decision envelope. After writing, reread state.json from disk and verify all work-path invariants before responding: current_item_id equals selected_id; exactly one item has status=in_progress; that sole in_progress item's id equals selected_id; that item's attempts equal previous_attempts + 1; no other item changed status; and the context.item.id you will emit equals selected_id. If any invariant fails, repair state.json and reread it again before responding. If the invariants still cannot be made true, choose `replan` rather than `work`, so Validate Item Plan can repair the queue. After the file is written and reread with all work-path invariants true, choose `work` with context_updates for context.item.id, context.item.title, context.item.summary, context.item.decision_ids, context.item.acceptance_criteria, context.item.constraints, context.item.target_paths, context.item.depends_on, context.item.attempts, plus cleared context.review.summary, context.review.required_changes, context.review.blockers, context.validation.item_id, context.validation.status, context.validation.summary, missing_prerequisites, validation_message, and validation_exit_code. Choose `work` only after state.json on disk agrees with the emitted context.item.* binding and the selected item is the sole in_progress item. If no pending or in_progress items remain, clear current_item_id in state.json before responding and choose `audit`. On the `audit` path, include context_updates that set the active item keys, review feedback keys, and validation scratch keys to JSON null so the live binding is fully cleared before milestone audit. Choose `audit` only after rereading state.json from disk and confirming current_item_id=null and no item is in_progress. If pending items remain but none are ready, keep the milestone state valid and choose `replan` so Validate Item Plan can restructure the stuck items. Respond with ONLY a JSON decision envelope.
