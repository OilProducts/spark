---
type: agent_task
title: Repair Evidence
contracts:
  reads_context:
  - context.action.id
  - context.action.summary
  writes_context:
  - context.action.repair
  response: status_envelope
routes:
- to: verify
  condition: outcome=success
---

The current action's verify.sh failed. Read .math-research/current-action and its evidence directory, especially verification.log. Make one focused repair to the evidence, program, formalization, or verifier. Never weaken a check, change the target claim silently, introduce axioms, or label unsupported output as evidence. If the claim cannot be repaired honestly, withdraw or downgrade it in claim.json and leave a verifier that accurately reflects the remaining machine-checkable claim. Respond with ONLY a JSON status envelope whose context_updates set context.action.repair.
