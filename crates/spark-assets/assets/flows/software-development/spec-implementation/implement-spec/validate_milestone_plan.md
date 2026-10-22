---
type: agent_task
title: Validate Milestone Plan
runtime: {}
contracts:
  response: status_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: next_milestone
  label: Validated
  condition: outcome=success
  extensions:
    loop_restart: false
- to: plan_milestones
  label: Replan
  condition: outcome=fail && preferred_label=Replan
  extensions:
    loop_restart: false
- to: plan_milestones
  label: Replan Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json. The decision ledger is .spark/spec-implementation/current/spec/contract-decisions.json. Validate the milestone, requirement, and decision ledgers before any long-running implementation work begins. Confirm milestone ids are unique, every milestone depends_on id exists, no milestone depends on itself, the milestone dependency graph is acyclic, every requirement milestone_id points to a real milestone, every milestone requirement_ids entry exists in the requirement ledger and stays consistent with the requirements assigned to that milestone, and requirement dependency ordering is compatible with milestone ordering. Confirm every milestone decision_id exists in .spark/spec-implementation/current/spec/contract-decisions.json, every relevant decision is covered by at least one milestone, and milestone acceptance criteria plus target_paths do not contradict their bound decisions. Cleanup milestones may use empty requirement_ids and empty decision_ids only when they add no requirement or decision coverage. Repair purely mechanical ledger inconsistencies in place when the intended decomposition is still clear, keeping the JSON files valid. If decision coverage is missing, a material conflict is unresolved, the decomposition is structurally wrong, or the plan cannot be repaired truthfully from the written ledgers, leave the files valid and return outcome fail with preferred_label Replan so milestone planning can be regenerated. If the plan is coherent after any needed repairs, return outcome success. Respond with ONLY a JSON status envelope.
