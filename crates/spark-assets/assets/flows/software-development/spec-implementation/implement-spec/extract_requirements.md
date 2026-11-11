---
type: agent_task
title: Extract Requirements
runtime: {}
contracts:
  reads_context:
  - context.request.spec_path
  - context.request.scope_hint
  - context.request.constraints
  - context.request.target_paths
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
    to: design_architecture
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: extract_requirements
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/source.md as the authoritative spec snapshot for this run. Do not reread context.request.spec_path or another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root) and <internal.project_work_dir>/spec-implementation/. Do not inspect paths outside it. Write <internal.project_work_dir>/spec-implementation/current/spec/requirements.json as valid JSON with a requirements array. Each requirement must include: id, spec_refs, title, summary, acceptance_criteria, dependencies, target_paths, status, milestone_id, and notes. Use status=pending for newly extracted requirements. Prefer precise, testable requirements over paraphrased prose. Preserve scope_hint, constraints, and target_paths intent from <internal.project_work_dir>/spec-implementation/current/state.json when deciding what is in scope. Choose `done` only after the requirements ledger is written. Respond with ONLY a JSON decision envelope.
