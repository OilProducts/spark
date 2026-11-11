---
type: agent_task
title: Record Milestone Success
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.requirement_ids
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: record_milestone_success
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.requirement_ids, the milestone state in <internal.project_work_dir>/spec-implementation/current/milestones/<context milestone id>/state.json, and <internal.project_work_dir>/spec-implementation/current/validation-result.json if it exists. Treat context.milestone.id plus milestone-local state as authoritative even if shared runtime milestone metadata drifted during implementation. Write <internal.project_work_dir>/spec-implementation/current/milestones/<context milestone id>/result.json with fields status, outcome, summary, completed_requirement_ids, and evidence setting the JSON field values "status": "completed" and "outcome": "success". The completed_requirement_ids list should come from context.milestone.requirement_ids. Choose `done` only after the result file is written. Respond with ONLY a JSON decision envelope.
