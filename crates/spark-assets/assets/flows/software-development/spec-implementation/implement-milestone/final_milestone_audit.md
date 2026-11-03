---
type: agent_task
title: Final Milestone Audit
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.title
  - context.milestone.objective
  - context.milestone.requirement_ids
  - context.milestone.decision_ids
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
    to: record_milestone_success
    description: The current item or milestone satisfies this node’s completion criteria with all required evidence and ledger updates; no in-scope work remains for that unit.
  extend:
    to: validate_item_plan
    description: Remaining implementable work belongs in the current scope; durably record the additional work required by this node’s prompt before continuing.
  unfinished:
    to: final_milestone_audit
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/spec/architecture.md, .spark/state/spec-implementation/current/spec/contract-decisions.json, the milestone state under .spark/state/spec-implementation/current/milestones/<context milestone id>/state.json, and the current repository state as needed. The decision ledger is .spark/state/spec-implementation/current/spec/contract-decisions.json. Treat context.milestone.* plus milestone-local state as the authoritative milestone identity for this child worker. Treat shared runtime binding files under .spark/state/spec-implementation/current/current-milestone.json and state.json.current_milestone_id as bookkeeping that may legitimately drift during implementation rather than as the authoritative child binding after dispatch. Treat .spark/state/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/state/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/state/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Do not reread another duplicate copy of the same spec from the repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Decide whether this milestone is now complete. A milestone is not complete while any decision bound to this milestone remains unmet, even if metadata or validation bookkeeping passes. Treat missing deliverability work inside this milestone as remaining implementable work, not premature success. If the milestone is complete, choose `complete`. If more implementable repository work remains inside the same milestone, append new pending items directly into milestone_dir/state.json using the existing item schema including decision_ids, the next unused milestone item ids, status=pending, and attempts=0; preserve every existing item''s id, status, attempts, and notes, and do not duplicate equivalent pending work. Choose `extend` only after the durable ledger contains the appended work. Use `extend` for remaining packaging, bootstrap, export, validation, repository-hygiene, cohesion cleanup, or unmet public-surface decision work that still belongs inside this milestone. Metadata-only repair items are allowed only when all bound decision_ids are already satisfied; otherwise append real implementation items for the unmet decision work. Ambiguity, under-specification, or a material spec-versus-architecture conflict missing from .spark/state/spec-implementation/current/spec/contract-decisions.json is not a reason to stop: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/state/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, then append the resulting work as pending items and choose `extend`. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, leaving the milestone state valid. Respond with ONLY a JSON decision envelope.
