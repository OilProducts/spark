---
type: agent_task
title: Audit Milestone
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.title
  - context.milestone.objective
  - context.milestone.requirement_ids
  - context.milestone.decision_ids
  - context.stack.child.status
  - context.stack.child.outcome
  - context.stack.child.outcome_reason_code
  - context.stack.child.outcome_reason_message
  writes_context:
  - context.milestone.id
  - context.milestone.title
  - context.milestone.objective
  - context.milestone.requirement_ids
  - context.milestone.decision_ids
  - context.milestone.acceptance_criteria
  - context.milestone.target_paths
  - context.milestone.attempts
  response: status_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: commit_milestone
  label: Commit
  condition: outcome=success
  extensions:
    loop_restart: false
- to: rewrite_milestones
  label: Rewrite Milestones
  condition: outcome=fail && preferred_label=Rewrite
  extensions:
    loop_restart: false
- to: run_milestone
  label: Extend
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
- to: rewrite_milestones
  label: Rewrite Milestones Partial
  condition: outcome=partial_success && preferred_label=Rewrite
  weight: 1
  extensions:
    loop_restart: false
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, .spark/spec-implementation/current/spec/architecture.md, .spark/spec-implementation/current/spec/contract-decisions.json, .spark/spec-implementation/current/spec/source.md, and .spark/spec-implementation/current/milestones/<context milestone id>/result.json if it exists. Also inspect context.stack.child.status, context.stack.child.outcome, context.stack.child.outcome_reason_code, and context.stack.child.outcome_reason_message. Treat context.milestone.* plus milestone-local child artifacts as the authoritative identity for the milestone under audit. If shared runtime binding files under .spark/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during the child run, judge those files as shared runtime bookkeeping rather than as the authoritative child control plane after dispatch. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved machine-readable resolution set for any material interpretation, naming, ownership, or validation decision. Judge the milestone against its requirement_ids and decision_ids. If the milestone is satisfactorily implemented, treat milestones.json as the authoritative milestone queue ledger: update milestones.json and requirements.json to mark the covered milestone and requirements done, clear .spark/spec-implementation/current/state.json.current_milestone_id, remove or clear .spark/spec-implementation/current/current-milestone.json, and context_updates include context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts set to JSON null so no completed milestone remains the live binding. Return outcome success only when the milestone's touched surfaces are honestly deliverable for their scope, including truthful validation/bootstrap expectations, coherent exports, docs, manifests, ignore rules, and conformance to the bound contract decisions, with no reliance on test-only bootstrap hacks or weaker internal-only validation paths as the primary way the touched work is used. Do not accept a milestone if it skipped the strongest deterministic repo-wide gate that was available, if claimed public surfaces were validated only through internal or bypass paths, if a bound decision remains unmet, or if remaining decision-bound implementation work was downgraded into metadata-only sync. If the milestone is mostly complete but bounded remaining work still belongs inside this same milestone, leave the milestone current and append one or more pending items directly to .spark/spec-implementation/current/milestones/<context milestone id>/state.json before returning. Preserve every existing item's id, status, attempts, and notes; do not delete, renumber, replace, or reopen completed items. Give each genuinely new item the next unused milestone item id, the full existing item schema, status=pending, attempts=0, and notes that identify the audit gap. Do not duplicate an equivalent pending item. Keep current_item_id null and no item in_progress at this audit handoff. Return outcome partial_success with preferred_label Extend only after the durable ledger contains bounded pending work that directly covers the audit finding so the milestone worker can continue. If the milestone decomposition is wrong or the remaining deliverability work should be replanned as bounded milestone-level follow-up, write .spark/spec-implementation/current/rewrite-milestones.json with replacement milestones and return outcome fail with preferred_label Rewrite. Do not silently choose between spec and architecture when a material conflict is missing from .spark/spec-implementation/current/spec/contract-decisions.json; instead choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, routing any resulting work through Extend or Rewrite. A failed or incomplete child run is a reason to Extend or Rewrite, not to stop. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, after updating the ledgers accordingly. Keep the decision tied to milestone completion and truthful deliverability, not optimism.
