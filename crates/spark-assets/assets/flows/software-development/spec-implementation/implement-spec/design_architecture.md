---
type: agent_task
title: Design Architecture
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
    to: evaluate_architecture
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: design_architecture
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/source.md and <internal.project_work_dir>/spec-implementation/current/spec/requirements.json. Treat <internal.project_work_dir>/spec-implementation/current/spec/source.md as the source of product behavior and external contract for this run. Do not reread another duplicate copy of the same spec from the repository. Read files needed from the current repository. Stay within the current cwd (the repository root) and <internal.project_work_dir>/spec-implementation/. Do not inspect paths outside it. Write <internal.project_work_dir>/spec-implementation/current/spec/architecture.md as the initial implementation architecture for the spec, and write <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json as valid JSON with a top-level decisions array. The durable contract ledger is <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, symlinked from specs/<slug>/. Each decision must include id, title, summary, requirement_ids, spec_refs, architecture_refs, behavioral_contract, implementation_contract, and validation_expectations. The architecture must define the canonical repository topology, implementation boundaries, documented public interface, validation strategy, repository hygiene expectations, and any dependencies between requirements. The contract-decisions file must capture every material spec-versus-architecture resolution, naming normalization, surface ownership choice, or validation expectation needed to implement the architecture truthfully. If no material decisions are needed, still write a valid file with decisions=[]. Avoid wrapper-only or duplicate delivery layers unless an external compatibility contract requires them. Avoid designs that rely on environment-specific hacks, test-only bootstrap behavior, or compatibility shims as the primary way the system works. Keep the plan grounded in the current repository and the written spec rather than idealized greenfield assumptions. Choose `done` only after both files are written. Respond with ONLY a JSON decision envelope.
