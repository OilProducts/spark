---
type: agent_task
title: Revise Architecture
runtime: {}
contracts:
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
    to: revise_architecture
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read .spark/state/spec-implementation/current/spec/architecture.md, .spark/state/spec-implementation/current/spec/contract-decisions.json, .spark/state/spec-implementation/current/spec/requirements.json, .spark/state/spec-implementation/current/spec/source.md, and .spark/state/spec-implementation/current/architecture-rework.md if it exists. Resolve each blocking finding in architecture-rework.md, keeping the implementation contract aligned with the product behavior in the spec. Resolve findings at the architecture level: correct the boundary, interface, sequencing, validation strategy, or contract decision the finding concerns, and add mechanisms or detail wherever a finding genuinely requires them. Prefer the simplest design that resolves a finding; restructuring or removing an over-specified mechanism is often a better fix than layering more rules onto it. Where a finding turns on subsystem-internal detail that does not change a boundary, an interface, the sequencing, or spec-visible behavior, it is acceptable to state the invariant the subsystem must uphold and leave the mechanism to milestone work rather than fully specifying it. Avoid changes unrelated to the findings, and preserve valid parts of the existing design instead of rewriting aimlessly. Update .spark/state/spec-implementation/current/spec/contract-decisions.json alongside the architecture so every material resolution remains explicit, complete, and machine-readable. Choose `done` only after both files are updated. Respond with ONLY a JSON decision envelope.
