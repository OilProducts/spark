---
type: agent_task
title: Extract Items
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
    to: validate_item_plan
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: extract_items
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/spec/architecture.md, and .spark/state/spec-implementation/current/spec/contract-decisions.json. The architecture file is .spark/state/spec-implementation/current/spec/architecture.md and the decision ledger is .spark/state/spec-implementation/current/spec/contract-decisions.json. Treat context.milestone.* plus milestone_dir/state.json as the authoritative child-worker milestone binding even if shared runtime bindings under .spark/state/spec-implementation/current/current-milestone.json or state.json.current_milestone_id later drift. Treat .spark/state/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/state/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/state/spec-implementation/current/spec/contract-decisions.json as the approved machine-readable resolution set. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Derive milestone_dir as .spark/state/spec-implementation/current/milestones/<context milestone id>/ and read milestone_dir/state.json before editing it. If its items array is nonempty, treat it as an existing initial or extension plan: preserve current_item_id and the entire items array exactly, including every id, title, status, attempts, dependency, and note, and do not regenerate, collapse, renumber, or replace any item. Choose `done` only after rereading the unchanged queue. Only when the items array is empty, update state.json with bounded implementable work items for this milestone. Each newly extracted item must include: id, title, summary, decision_ids, acceptance_criteria, constraints, target_paths, depends_on, status, attempts, and notes, using status=pending and attempts=0. Every milestone decision_id must appear in at least one item's decision_ids, and each bound decision must be reflected directly in item acceptance_criteria and target_paths rather than buried only in notes. If the spec, architecture, and contract decisions cannot be reconciled into a truthful initial item plan, do not silently choose one source: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/state/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis. Preserve milestone metadata and keep the file valid JSON. Choose `done` only after the milestone state is written or the existing nonempty queue is verified unchanged. Respond with ONLY a JSON decision envelope.
