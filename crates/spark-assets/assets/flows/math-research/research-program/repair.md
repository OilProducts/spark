---
type: agent_task
title: Repair Evidence
contracts:
  reads_context:
  - context.action.id
  - context.action.summary
  writes_context:
  - context.action.repair
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: verify
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: repair
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

The current action's evidence check was rejected: its claim.json is missing or malformed, or its verify.sh failed. Read .math-research/current-action and its evidence directory, especially verification.log. Make one focused repair to the evidence, program, formalization, or verifier. Never weaken a check, change the target claim silently, introduce axioms, or label unsupported output as evidence. If the claim cannot be repaired honestly, withdraw or downgrade it in claim.json and leave a verifier that accurately reflects the remaining machine-checkable claim. Set context_updates for context.action.repair, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
