---
type: agent_task
title: Repair Statement
contracts:
  reads_context:
  - context.arbiter.decision
  writes_context:
  - context.statement.current
  response: status_envelope
routes:
- to: round_gate
---

You are the statement surgeon of the prove/refute session. The arbiter routed here because the current statement version needs surgery; read context.arbiter.decision, shared/statement.md, shared/exchange.md, and both branch reports. Append a new numbered statement version to shared/statement.md — never rewrite earlier versions — stating the revised claim with every definition, quantifier, and range pinned, the exact edit relative to the previous version, and the evidence that forced it. A repair may weaken (exclude a counterexample family by an honest structural hypothesis, never by enumerating away the known witnesses) or strengthen (adopt the stronger or corrected claim the proof attempt actually established). The revision must be checked, not guessed: verify computationally that it survives every verified counterexample and near-miss recorded so far — a small script or direct evaluation under shared/ is fine — and record the check you ran in the new version's section. Delete any prover/PROVED or refuter/REFUTED markers, since they refer to the previous statement version. Respond with ONLY a JSON status envelope whose context_updates set context.statement.current.
