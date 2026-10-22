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
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: plan_current
  label: Replan
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: validate_active_item_state
  label: Check Item State
  condition: outcome=success
  extensions:
    loop_restart: false
---

Implement the current milestone item described by context.item.summary, context.item.acceptance_criteria, context.item.constraints, and context.item.target_paths. Keep the scope bounded to this item only. If review feedback exists in context.review.summary or context.review.required_changes, address it directly in this pass before validation. Do not rely on environment-specific hacks, test-only bootstrap behavior, or compatibility shims as the primary way the item works. Run focused checks while implementing, but do not run the repository-wide validation command; the downstream validate_repo node owns that gate. Do not edit milestone_dir/state.json or .spark/spec-implementation/current/validation-result.json in this node. Do not claim that a later item is now active, complete, or next; queue advancement belongs only to the queue-state nodes. If the cleanest path appears broader than the current item, keep the repository coherent so review can request Rewrite instead of cementing a brittle workaround.
