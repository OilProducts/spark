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
  response: status_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: design_architecture
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/spec/source.md as the authoritative spec snapshot for this run. Do not reread context.request.spec_path or another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Write .spark/spec-implementation/current/spec/requirements.json as valid JSON with a requirements array. Each requirement must include: id, spec_refs, title, summary, acceptance_criteria, dependencies, target_paths, status, milestone_id, and notes. Use status=pending for newly extracted requirements. Prefer precise, testable requirements over paraphrased prose. Preserve scope_hint, constraints, and target_paths intent from .spark/spec-implementation/current/state.json when deciding what is in scope. Respond with ONLY a JSON status envelope and return outcome success only after the requirements ledger is written.
