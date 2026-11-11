---
type: agent_task
title: Plan Cleanup Milestone
runtime: {}
contracts:
  reads_context:
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate_milestone_plan
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: plan_cleanup_milestone
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/repository-integrity-gaps.md, <internal.project_work_dir>/spec-implementation/current/spec/source.md, <internal.project_work_dir>/spec-implementation/current/spec/requirements.json, <internal.project_work_dir>/spec-implementation/current/milestones.json, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. Append or revise the pending cleanup/refactor milestones needed to close the documented repository-integrity gaps, using a single milestone when it can cover them coherently. Preserve completed and in_progress milestones exactly as they are. If an earlier pending cleanup/refactor milestone already exists for the same final-gap work, revise it instead of appending a duplicate. Each cleanup milestone must use the milestone schema including decision_ids, and may use an empty requirement_ids list plus empty decision_ids only when the work is repository-integrity-only and does not add new requirement or decision coverage. Keep milestones.json valid JSON. Choose `done` only after the cleanup milestone plan is updated. Respond with ONLY a JSON decision envelope.
