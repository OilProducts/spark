---
type: agent_task
title: Final Conformance Audit
runtime: {}
contracts:
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  extend:
    to: plan_milestones
    description: Remaining implementable work belongs in the current scope; durably record the additional work required by this node’s prompt before continuing.
  requirements_covered:
    to: repository_integrity_audit
    description: All in-scope requirements and bound contract decisions are covered; repository deliverability still requires its separate audit.
  unfinished:
    to: final_conformance_audit
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/milestones.json, .spark/state/spec-implementation/current/spec/architecture.md, and .spark/state/spec-implementation/current/spec/contract-decisions.json. Treat .spark/state/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/state/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/state/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Read the current repository state only as needed to judge whether all in-scope requirements are now covered. Treat unresolved contract decisions that are bound to in-scope requirements as uncovered requirement work, not as cosmetic follow-up. If all in-scope requirements and their bound decisions are covered, choose `requirements_covered`. If uncovered but implementable requirement or decision work remains, append or revise milestones in .spark/state/spec-implementation/current/milestones.json and choose `extend`. Milestones appended here must use the milestone schema including decision_ids. Ambiguity or an unresolved design issue is not a reason to stop: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/state/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis, then plan the remaining work through `extend`. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving, leaving the ledgers valid. Keep this decision tied to requirement coverage only; any remaining deliverability or repository-integrity cleanup belongs to repository_integrity_audit. Respond with ONLY a JSON decision envelope.
