---
type: agent_task
title: Plan Cleanup Milestone
runtime: {}
contracts:
  response: status_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: validate_milestone_plan
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/repository-integrity-gaps.md, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json. Append or revise the pending cleanup/refactor milestones needed to close the documented repository-integrity gaps, using a single milestone when it can cover them coherently. Preserve completed and in_progress milestones exactly as they are. If an earlier pending cleanup/refactor milestone already exists for the same final-gap work, revise it instead of appending a duplicate. Each cleanup milestone must use the milestone schema including decision_ids, and may use an empty requirement_ids list plus empty decision_ids only when the work is repository-integrity-only and does not add new requirement or decision coverage. Keep milestones.json valid JSON and respond with ONLY a JSON status envelope. Return outcome success only after the cleanup milestone plan is updated.
