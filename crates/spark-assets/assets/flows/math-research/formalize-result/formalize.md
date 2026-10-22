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
  response: status_envelope
routes:
- to: lean_gate
---

Read DOSSIER.md first per AGENTS.md dossier discipline: prior sessions may already record proved lemmas, failed proof strategies, or statement caveats for this result. Then formalize the result stated in context.request.statement in Lean 4 inside the lean/ project (its Mathlib is prebuilt; never modify /opt/mathlab/mathlib4). Consult context.request.reference for the intended proof when present. Add the formal statement and its proof under lean/MathLab/, imported from lean/MathLab.lean, and record every theorem name you consider part of the deliverable in .mathlab/theorems.txt, one fully qualified name per line. Follow AGENTS.md proof-integrity rules exactly: no sorry or admit, no new axiom declarations, and if you must weaken or adjust the statement to make it provable, state the deviation explicitly in .mathlab/notes.md. If .mathlab/lean-gate.log exists, it holds the compiler output of a failed previous gate; if context.review.required_changes is present, it lists fidelity gaps a verifier found; close both. Verify locally with (cd lean && lake build) before finishing, then append a session entry to DOSSIER.md per its structure — statement formalized, theorem names, any declared deviations, and failed proof strategies with what they taught; dossier entries are append-only. Propose the natural next session in a closing section of findings.md. Respond with ONLY a JSON status envelope whose context_updates set context.formalization.summary and context.formalization.theorems (the recorded theorem names).
