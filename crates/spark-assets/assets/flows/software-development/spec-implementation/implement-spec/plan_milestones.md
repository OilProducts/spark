---
type: agent_task
title: Plan Milestones
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

Read .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract, .spark/spec-implementation/current/spec/architecture.md as the binding implementation contract, and .spark/spec-implementation/current/spec/contract-decisions.json as the approved resolution set. Also read .spark/spec-implementation/current/architecture-open-items.md if it exists; it lists deferrable architecture-review findings, and each item must be assigned to the milestone that will settle it, reflected in that milestone's acceptance criteria or notes. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Write .spark/spec-implementation/current/milestones.json as valid JSON with a milestones array. Each milestone must include: id, title, objective, requirement_ids, decision_ids, acceptance_criteria, target_paths, depends_on, status, attempts, and notes. The milestones should be large enough to matter architecturally but small enough to implement and verify coherently. Use status=pending and attempts=0 for new milestones. Every relevant contract decision must appear in at least one milestone's decision_ids, and milestone acceptance criteria plus target_paths must reflect those bound decisions directly rather than leaving them only in notes. Do not simply echo the requirements ledger; group requirements and decisions into meaningful delivery slices. Respond with ONLY a JSON status envelope and return outcome success only after the milestone plan is written.
