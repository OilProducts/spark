---
type: agent_task
title: Prepare Milestone State
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.title
  - context.milestone.objective
  - context.milestone.requirement_ids
  - context.milestone.decision_ids
  - context.milestone.acceptance_criteria
  - context.milestone.target_paths
  - context.milestone.attempts
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: extract_items
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: prepare_milestone_state
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, and context.milestone.attempts, then derive milestone_dir as <internal.project_work_dir>/spec-implementation/current/milestones/<context milestone id>/. Treat the bound context.milestone.* snapshot as authoritative for this child even if shared runtime binding files under <internal.project_work_dir>/spec-implementation/current/current-milestone.json later drift. Create milestone_dir if it does not exist. If milestone_dir/state.json does not exist, write it as valid JSON with this shape: {"milestone":{...current milestone snapshot including decision_ids...},"current_item_id":null,"items":[],"updated_at":"<timestamp>"}. If state.json already exists and is valid for this same milestone, preserve current_item_id and the entire items array exactly, including every item's id, title, status, attempts, dependencies, and notes; never replace an existing item array with an empty array. Refresh only the milestone metadata from the bound snapshot and updated_at as needed. If the existing file belongs to another milestone or is invalid, do not discard its history: move it aside within milestone_dir to state.<UTC timestamp>.archived.json, then write a fresh state.json for the bound milestone as above and continue. Choose `done` only after the milestone state file exists with any existing queue history preserved. Respond with ONLY a JSON decision envelope.
