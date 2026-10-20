---
type: agent_task
title: Revise Architecture
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

Read .spark/spec-implementation/current/spec/architecture.md, .spark/spec-implementation/current/spec/contract-decisions.json, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/spec/source.md, and .spark/spec-implementation/current/architecture-rework.md if it exists. Resolve each blocking finding in architecture-rework.md, keeping the implementation contract aligned with the product behavior in the spec. Resolve findings at the architecture level: correct the boundary, interface, sequencing, validation strategy, or contract decision the finding concerns, and add mechanisms or detail wherever a finding genuinely requires them. Prefer the simplest design that resolves a finding; restructuring or removing an over-specified mechanism is often a better fix than layering more rules onto it. Where a finding turns on subsystem-internal detail that does not change a boundary, an interface, the sequencing, or spec-visible behavior, it is acceptable to state the invariant the subsystem must uphold and leave the mechanism to milestone work rather than fully specifying it. Avoid changes unrelated to the findings, and preserve valid parts of the existing design instead of rewriting aimlessly. Update .spark/spec-implementation/current/spec/contract-decisions.json alongside the architecture so every material resolution remains explicit, complete, and machine-readable. Respond with ONLY a JSON status envelope and return outcome success only after both files are updated.
