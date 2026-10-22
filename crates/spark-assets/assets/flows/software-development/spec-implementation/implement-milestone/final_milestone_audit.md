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
- to: record_milestone_success
  label: Complete
  condition: outcome=success
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Extend
  condition: outcome=fail && preferred_label=Extend
  extensions:
    loop_restart: false
- to: validate_item_plan
  label: Extend Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/spec/architecture.md, .spark/spec-implementation/current/spec/contract-decisions.json, the milestone state under .spark/spec-implementation/current/milestones/<context milestone id>/state.json, and the current repository state as needed. The decision ledger is .spark/spec-implementation/current/spec/contract-decisions.json. Treat context.milestone.* plus milestone-local state as the authoritative milestone identity for this child worker. Treat shared runtime binding files under .spark/spec-implementation/current/current-milestone.json and state.json.current_milestone_id as bookkeeping that may legitimately drift during implementation rather than as the authoritative child binding after dispatch. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Do not reread another duplicate copy of the same spec from the repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Decide whether this milestone is now complete. A milestone is not complete while any decision bound to this milestone remains unmet, even if metadata or validation bookkeeping passes. Treat missing deliverability work inside this milestone as remaining implementable work, not premature success. If the milestone is complete, respond with ONLY a JSON status envelope with outcome success. If more implementable repository work remains inside the same milestone, append new pending items directly into milestone_dir/state.json using the existing item schema including decision_ids, the next unused milestone item ids, status=pending, and attempts=0; preserve every existing item''s id, status, attempts, and notes, and do not duplicate equivalent pending work. Return outcome fail with preferred_label Extend only after the durable ledger contains the appended work. Use Extend for remaining packaging, bootstrap, export, validation, repository-hygiene, cohesion cleanup, or unmet public-surface decision work that still belongs inside this milestone. Metadata-only repair items are allowed only when all bound decision_ids are already satisfied; otherwise append real implementation items for the unmet decision work. Ambiguity, under-specification, or a material spec-versus-architecture conflict missing from .spark/spec-implementation/current/spec/contract-decisions.json is not a reason to stop: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, then append the resulting work as pending items and return Extend. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, leaving the milestone state valid.
