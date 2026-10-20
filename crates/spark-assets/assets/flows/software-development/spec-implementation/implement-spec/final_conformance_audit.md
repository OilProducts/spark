---
type: agent_task
title: Final Conformance Audit
runtime: {}
contracts:
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
- to: plan_milestones
  label: Extend
  condition: outcome=fail && preferred_label=Extend
  extensions:
    loop_restart: false
- to: plan_milestones
  label: Extend Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: repository_integrity_audit
  label: Requirements Covered
  condition: outcome=success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Read the current repository state only as needed to judge whether all in-scope requirements are now covered. Treat unresolved contract decisions that are bound to in-scope requirements as uncovered requirement work, not as cosmetic follow-up. If all in-scope requirements and their bound decisions are covered, respond with ONLY a JSON status envelope with outcome success. If uncovered but implementable requirement or decision work remains, append or revise milestones in .spark/spec-implementation/current/milestones.json and return outcome fail with preferred_label Extend. Milestones appended here must use the milestone schema including decision_ids. Ambiguity or an unresolved design issue is not a reason to stop: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, then plan the remaining work through Extend. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, leaving the ledgers valid. Keep this decision tied to requirement coverage only; any remaining deliverability or repository-integrity cleanup belongs to repository_integrity_audit.
