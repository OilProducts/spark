---
type: agent_task
title: Repair Statement
contracts:
  reads_context:
  - context.arbiter.decision
  writes_context:
  - context.statement.current
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: round_gate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: repair_statement
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

You are the statement surgeon of the prove/refute session. The arbiter routed here because the current statement version needs surgery; read context.arbiter.decision, shared/statement.md, shared/exchange.md, and both branch reports. Append a new numbered statement version to shared/statement.md — never rewrite earlier versions — stating the revised claim with every definition, quantifier, and range pinned, the exact edit relative to the previous version, and the evidence that forced it. A repair may weaken (exclude a counterexample family by an honest structural hypothesis, never by enumerating away the known witnesses) or strengthen (adopt the stronger or corrected claim the proof attempt actually established). The revision must be checked, not guessed: verify computationally that it survives every verified counterexample and near-miss recorded so far — a small script or direct evaluation under shared/ is fine — and record the check you ran in the new version's section. Delete any prover/PROVED or refuter/REFUTED markers, since they refer to the previous statement version. Set context_updates for context.statement.current, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
