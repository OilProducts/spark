---
type: agent_task
title: Formalize
contracts:
  reads_context:
  - context.request.statement
  - context.request.reference
  - context.review.required_changes
  writes_context:
  - context.formalization.summary
  - context.formalization.theorems
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: lean_gate
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: formalize
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read DOSSIER.md first per AGENTS.md dossier discipline: prior sessions may already record proved lemmas, failed proof strategies, or statement caveats for this result. Then formalize the result stated in context.request.statement in Lean 4 inside the lean/ project (its Mathlib is prebuilt; never modify /opt/mathlab/mathlib4). Consult context.request.reference for the intended proof when present. Add the formal statement and its proof under lean/MathLab/, imported from lean/MathLab.lean, and record every theorem name you consider part of the deliverable in .mathlab/theorems.txt, one fully qualified name per line. Follow AGENTS.md proof-integrity rules exactly: no sorry or admit, no new axiom declarations, and if you must weaken or adjust the statement to make it provable, state the deviation explicitly in .mathlab/notes.md. If .mathlab/lean-gate.log exists, it holds the compiler output of a failed previous gate; if context.review.required_changes is present, it lists fidelity gaps a verifier found; close both. Verify locally with (cd lean && lake build) before finishing, then append a session entry to DOSSIER.md per its structure — statement formalized, theorem names, any declared deviations, and failed proof strategies with what they taught; dossier entries are append-only. Propose the natural next session in a closing section of findings.md. Set context_updates for context.formalization.summary and context.formalization.theorems (the recorded theorem names), then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
