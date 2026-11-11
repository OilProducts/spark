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
  - internal.project_work_dir
  writes_context:
  - context.milestone.id
  - context.milestone.title
  - context.milestone.objective
  - context.milestone.requirement_ids
  - context.milestone.decision_ids
  - context.milestone.acceptance_criteria
  - context.milestone.target_paths
  - context.milestone.attempts
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  complete:
    to: commit_milestone
    description: The current item or milestone satisfies this node’s completion criteria with all required evidence and ledger updates; no in-scope work remains for that unit.
  extend:
    to: run_milestone
    description: Remaining implementable work belongs in the current scope; durably record the additional work required by this node’s prompt before continuing.
  rewrite:
    to: rewrite_milestones
    description: The current item or milestone decomposition is unsuitable; write the required replacement-work ledger before rewriting it.
  unfinished:
    to: audit_milestone
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, <internal.project_work_dir>/spec-implementation/current/spec/requirements.json, <internal.project_work_dir>/spec-implementation/current/milestones.json, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, <internal.project_work_dir>/spec-implementation/current/spec/source.md, and <internal.project_work_dir>/spec-implementation/current/milestones/<context milestone id>/result.json if it exists. Also inspect context.stack.child.status, context.stack.child.outcome, context.stack.child.outcome_reason_code, and context.stack.child.outcome_reason_message. Treat context.milestone.* plus milestone-local child artifacts as the authoritative identity for the milestone under audit. If shared runtime binding files under <internal.project_work_dir>/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during the child run, judge those files as shared runtime bookkeeping rather than as the authoritative child control plane after dispatch. Treat <internal.project_work_dir>/spec-implementation/current/spec/source.md as the source of product behavior and external contract, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md as the binding implementation contract, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json as the approved machine-readable resolution set for any material interpretation, naming, ownership, or validation decision. Judge the milestone against its requirement_ids and decision_ids. If the milestone is satisfactorily implemented, treat milestones.json as the authoritative milestone queue ledger: update milestones.json and requirements.json to mark the covered milestone and requirements done, clear <internal.project_work_dir>/spec-implementation/current/state.json.current_milestone_id, remove or clear <internal.project_work_dir>/spec-implementation/current/current-milestone.json, and context_updates include context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts set to JSON null so no completed milestone remains the live binding. Choose `complete` only when the milestone's touched surfaces are honestly deliverable for their scope, including truthful validation/bootstrap expectations, coherent exports, docs, manifests, ignore rules, and conformance to the bound contract decisions, with no reliance on test-only bootstrap hacks or weaker internal-only validation paths as the primary way the touched work is used. Do not accept a milestone if it skipped the strongest deterministic repo-wide gate that was available, if claimed public surfaces were validated only through internal or bypass paths, if a bound decision remains unmet, or if remaining decision-bound implementation work was downgraded into metadata-only sync. If the milestone is mostly complete but bounded remaining work still belongs inside this same milestone, leave the milestone current and append one or more pending items directly to <internal.project_work_dir>/spec-implementation/current/milestones/<context milestone id>/state.json before returning. Preserve every existing item's id, status, attempts, and notes; do not delete, renumber, replace, or reopen completed items. Give each genuinely new item the next unused milestone item id, the full existing item schema, status=pending, attempts=0, and notes that identify the audit gap. Do not duplicate an equivalent pending item. Keep current_item_id null and no item in_progress at this audit handoff. Choose `extend` only after the durable ledger contains bounded pending work that directly covers the audit finding so the milestone worker can continue. If the milestone decomposition is wrong or the remaining deliverability work should be replanned as bounded milestone-level follow-up, write <internal.project_work_dir>/spec-implementation/current/rewrite-milestones.json with replacement milestones and choose `rewrite`. Do not silently choose between spec and architecture when a material conflict is missing from <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json; instead choose the most defensible interpretation consistent with the spec, record it as a new decision in <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, routing any resulting work through `extend` or `rewrite`. A blocked or incomplete child run is a reason to choose `extend` or `rewrite`, not to stop. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, after updating the ledgers accordingly. Keep the decision tied to milestone completion and truthful deliverability, not optimism. Respond with ONLY a JSON decision envelope.
