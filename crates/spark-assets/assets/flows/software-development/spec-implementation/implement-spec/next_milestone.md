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
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  audit:
    to: final_conformance_audit
    description: No pending or active work remains in this queue; clear the required live bindings and advance to its independent audit.
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  replan:
    to: validate_milestone_plan
    description: The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning.
  unfinished:
    to: next_milestone
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
  work:
    to: run_milestone
    description: A ready pending item or milestone has been durably dispatched with the required identity, attempt and context invariants verified.
---

Read .spark/state/spec-implementation/current/milestones.json as the authoritative milestone queue ledger. Read .spark/state/spec-implementation/current/state.json and .spark/state/spec-implementation/current/current-milestone.json only to keep those binding artifacts aligned with milestones.json; do not let stale binding files block progress when milestones.json truthfully shows the current milestone is already completed. Before choosing the next milestone, mechanically repair stale shared runtime bindings when the intended state is clear from milestones.json: if current_milestone_id or current-milestone.json points to a completed milestone, clear both; if they disagree with the ready pending milestone you are about to dispatch, rewrite them to match that milestone. A ready milestone is pending and all of its depends_on milestones are done. If a ready milestone exists, mark it in_progress, increment its attempts, set current_milestone_id in .spark/state/spec-implementation/current/state.json, write .spark/state/spec-implementation/current/current-milestone.json for that milestone, and choose `work` with context_updates for context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts. If no pending or in_progress milestones remain, clear current_milestone_id, remove or clear .spark/state/spec-implementation/current/current-milestone.json, and choose `audit`, with context_updates that set context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts to JSON null. If milestones remain but none are ready after any mechanical binding repair, repair the plan in .spark/state/spec-implementation/current/milestones.json: re-scope, split, reorder, or replace the stuck pending milestones so the remaining work can proceed, preserving every completed milestone and its history exactly, then keep the ledgers valid and choose `replan` so the repaired plan is revalidated. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Respond with ONLY a JSON decision envelope.
