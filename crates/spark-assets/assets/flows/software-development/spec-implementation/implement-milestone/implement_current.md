---
type: agent_task
title: Implement Current Item
runtime: {}
contracts:
  reads_context:
  - context.item.id
  - context.item.title
  - context.item.summary
  - context.item.acceptance_criteria
  - context.item.constraints
  - context.item.target_paths
  - context.item.attempts
  - context.review.summary
  - context.review.required_changes
  - context.review.blockers
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate_active_item_state
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  replan:
    to: plan_current
    description: The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning.
  unfinished:
    to: implement_current
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Implement the current milestone item described by context.item.summary, context.item.acceptance_criteria, context.item.constraints, and context.item.target_paths. Keep the scope bounded to this item only. If review feedback exists in context.review.summary or context.review.required_changes, address it directly in this pass before validation. Do not rely on environment-specific hacks, test-only bootstrap behavior, or compatibility shims as the primary way the item works. Run focused checks while implementing, but do not run the repository-wide validation command; the downstream validate_repo node owns that gate. Do not edit milestone_dir/state.json or <internal.project_work_dir>/spec-implementation/current/validation-result.json in this node. Do not claim that a later item is now active, complete, or next; queue advancement belongs only to the queue-state nodes. If the cleanest path appears broader than the current item, keep the repository coherent so review can choose `rewrite` instead of cementing a brittle workaround. Choose `done` when this bounded implementation pass is complete and ready for the active-item state check. Choose `replan` when the current item plan cannot honestly support implementation, recording the evidence in notes. Respond with ONLY a JSON decision envelope.
