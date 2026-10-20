---
type: agent_task
title: Design Architecture
runtime: {}
contracts:
  response: status_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: evaluate_architecture
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/spec/source.md and .spark/spec-implementation/current/spec/requirements.json. Treat .spark/spec-implementation/current/spec/source.md as the source of product behavior and external contract for this run. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root). Do not inspect paths outside it. Write .spark/spec-implementation/current/spec/architecture.md as the initial implementation architecture for the spec, and write .spark/spec-implementation/current/spec/contract-decisions.json as valid JSON with a top-level decisions array. The durable contract ledger is .spark/spec-implementation/current/spec/contract-decisions.json, symlinked from specs/<slug>/. Each decision must include id, title, summary, requirement_ids, spec_refs, architecture_refs, behavioral_contract, implementation_contract, and validation_expectations. The architecture must define the canonical repository topology, implementation boundaries, documented public interface, validation strategy, repository hygiene expectations, and any dependencies between requirements. The contract-decisions file must capture every material spec-versus-architecture resolution, naming normalization, surface ownership choice, or validation expectation needed to implement the architecture truthfully. If no material decisions are needed, still write a valid file with decisions=[]. Avoid wrapper-only or duplicate delivery layers unless an external compatibility contract requires them. Avoid designs that rely on environment-specific hacks, test-only bootstrap behavior, or compatibility shims as the primary way the system works. Keep the plan grounded in the current repository and the written spec rather than idealized greenfield assumptions. Respond with ONLY a JSON status envelope and return outcome success only after both files are written.
