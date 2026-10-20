---
type: agent_task
title: Next Milestone
runtime: {}
contracts:
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
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: final_conformance_audit
  label: Audit
  condition: outcome=success && preferred_label=Audit
  extensions:
    loop_restart: false
- to: run_milestone
  label: Work
  condition: outcome=success && preferred_label=Work
  extensions:
    loop_restart: false
- to: validate_milestone_plan
  label: Replan
  condition: outcome=fail && preferred_label=Replan
  extensions:
    loop_restart: false
- to: validate_milestone_plan
  label: Revalidate Plan Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
- to: final_conformance_audit
  label: Audit Partial
  condition: outcome=partial_success && preferred_label=Audit
  weight: 1
  extensions:
    loop_restart: false
- to: run_milestone
  label: Work Partial
  condition: outcome=partial_success && preferred_label=Work
  weight: 1
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/milestones.json as the authoritative milestone queue ledger. Read .spark/spec-implementation/current/state.json and .spark/spec-implementation/current/current-milestone.json only to keep those binding artifacts aligned with milestones.json; do not let stale binding files block progress when milestones.json truthfully shows the current milestone is already completed. Before choosing the next milestone, mechanically repair stale shared runtime bindings when the intended state is clear from milestones.json: if current_milestone_id or current-milestone.json points to a completed milestone, clear both; if they disagree with the ready pending milestone you are about to dispatch, rewrite them to match that milestone. A ready milestone is pending and all of its depends_on milestones are done. If a ready milestone exists, mark it in_progress, increment its attempts, set current_milestone_id in .spark/spec-implementation/current/state.json, write .spark/spec-implementation/current/current-milestone.json for that milestone, and respond with ONLY a JSON status envelope with outcome success, preferred_label Work, and context_updates for context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts. If no pending or in_progress milestones remain, clear current_milestone_id, remove or clear .spark/spec-implementation/current/current-milestone.json, and return outcome success with preferred_label Audit, with context_updates that set context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts to JSON null. If milestones remain but none are ready after any mechanical binding repair, repair the plan in .spark/spec-implementation/current/milestones.json: re-scope, split, reorder, or replace the stuck pending milestones so the remaining work can proceed, preserving every completed milestone and its history exactly, then keep the ledgers valid and return outcome fail with preferred_label Replan so the repaired plan is revalidated. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving.
