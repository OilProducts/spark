---
type: agent_task
title: Plan Milestones
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
    to: plan_milestones
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/source.md, <internal.project_work_dir>/spec-implementation/current/spec/requirements.json, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. Treat <internal.project_work_dir>/spec-implementation/current/spec/source.md as the source of product behavior and external contract, <internal.project_work_dir>/spec-implementation/current/spec/architecture.md as the binding implementation contract, and <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Also read <internal.project_work_dir>/spec-implementation/current/architecture-open-items.md if it exists; it lists deferrable architecture-review findings, and each item must be assigned to the milestone that will settle it, reflected in that milestone's acceptance criteria or notes. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root) and <internal.project_work_dir>/spec-implementation/. Do not inspect paths outside it. Write <internal.project_work_dir>/spec-implementation/current/milestones.json as valid JSON with a milestones array. Each milestone must include: id, title, objective, requirement_ids, decision_ids, acceptance_criteria, target_paths, depends_on, status, attempts, and notes. The milestones should be large enough to matter architecturally but small enough to implement and verify coherently. Use status=pending and attempts=0 for new milestones. Every relevant contract decision must appear in at least one milestone's decision_ids, and milestone acceptance criteria plus target_paths must reflect those bound decisions directly rather than leaving them only in notes. Do not simply echo the requirements ledger; group requirements and decisions into meaningful delivery slices. Choose `done` only after the milestone plan is written. Respond with ONLY a JSON decision envelope.
