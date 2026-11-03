---
type: agent_task
title: Validate Milestone Plan
runtime: {}
contracts:
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  replan:
    to: plan_milestones
    description: The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning.
  unfinished:
    to: validate_milestone_plan
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  validated:
    to: next_milestone
    description: The current queue or completion state passes this node’s identity, dependency, coverage and validation checks after any permitted repairs.
---

Read .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/milestones.json, .spark/state/spec-implementation/current/spec/architecture.md, and .spark/state/spec-implementation/current/spec/contract-decisions.json. The decision ledger is .spark/state/spec-implementation/current/spec/contract-decisions.json. Validate the milestone, requirement, and decision ledgers before any long-running implementation work begins. Confirm milestone ids are unique, every milestone depends_on id exists, no milestone depends on itself, the milestone dependency graph is acyclic, every requirement milestone_id points to a real milestone, every milestone requirement_ids entry exists in the requirement ledger and stays consistent with the requirements assigned to that milestone, and requirement dependency ordering is compatible with milestone ordering. Confirm every milestone decision_id exists in .spark/state/spec-implementation/current/spec/contract-decisions.json, every relevant decision is covered by at least one milestone, and milestone acceptance criteria plus target_paths do not contradict their bound decisions. Cleanup milestones may use empty requirement_ids and empty decision_ids only when they add no requirement or decision coverage. Repair purely mechanical ledger inconsistencies in place when the intended decomposition is still clear, keeping the JSON files valid. If decision coverage is missing, a material conflict is unresolved, the decomposition is structurally wrong, or the plan cannot be repaired truthfully from the written ledgers, leave the files valid and choose `replan` so milestone planning can be regenerated. If the plan is coherent after any needed repairs, choose `validated`. Respond with ONLY a JSON decision envelope.
