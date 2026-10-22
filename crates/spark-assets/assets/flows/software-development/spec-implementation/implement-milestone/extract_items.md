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
  response: status_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: validate_item_plan
  extensions:
    loop_restart: false
---

Read context.milestone.id, context.milestone.title, context.milestone.objective, context.milestone.requirement_ids, context.milestone.decision_ids, context.milestone.acceptance_criteria, context.milestone.target_paths, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json. The architecture file is .spark/spec-implementation/current/spec/architecture.md and the decision ledger is .spark/spec-implementation/current/spec/contract-decisions.json. Treat context.milestone.* plus milestone_dir/state.json as the authoritative child-worker milestone binding even if shared runtime bindings under .spark/spec-implementation/current/current-milestone.json or state.json.current_milestone_id later drift. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved machine-readable resolution set. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Derive milestone_dir as .spark/spec-implementation/current/milestones/<context milestone id>/ and read milestone_dir/state.json before editing it. If its items array is nonempty, treat it as an existing initial or extension plan: preserve current_item_id and the entire items array exactly, including every id, title, status, attempts, dependency, and note, and do not regenerate, collapse, renumber, or replace any item. Return outcome success only after rereading the unchanged queue. Only when the items array is empty, update state.json with bounded implementable work items for this milestone. Each newly extracted item must include: id, title, summary, decision_ids, acceptance_criteria, constraints, target_paths, depends_on, status, attempts, and notes, using status=pending and attempts=0. Every milestone decision_id must appear in at least one item's decision_ids, and each bound decision must be reflected directly in item acceptance_criteria and target_paths rather than buried only in notes. If the spec, architecture, and contract decisions cannot be reconciled into a truthful initial item plan, do not silently choose one source: choose the most defensible interpretation consistent with the spec, record it as a new decision in .spark/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and proceed on that basis. Preserve milestone metadata and keep the file valid JSON. Respond with ONLY a JSON status envelope and return outcome success only after the milestone state is written or the existing nonempty queue is verified unchanged.
