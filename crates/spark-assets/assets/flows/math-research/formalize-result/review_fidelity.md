---
type: agent_task
title: Review Fidelity
role: adversarial-reviewer
contracts:
  reads_context:
  - context.request.statement
  - context.formalization.theorems
  writes_context:
  - context.review.required_changes
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  faithful:
    to: commit_result
    description: The completed fidelity review accepts the formal statement or its explicitly declared deviations under the prompt’s fidelity criteria.
  revise:
    to: formalize
    description: The completed review requires revision of its subject within the existing scope; record the specific corrections and their evidence.
  unfinished:
    to: review_fidelity
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a Lean formalization. The build already passed the kernel gate; your job is the part the kernel cannot check: does the formal statement faithfully encode context.request.statement? Read the theorems named in .mathlab/theorems.txt inside lean/, and for each one check quantifier structure, hypotheses, definitional choices, and edge conditions against the informal claim — a proved theorem about the wrong statement is a violation, not a result. Run (cd lean && lake env lean) probes or #check/#print inspections as needed. Verify any stated deviations in .mathlab/notes.md are honest and prominently declared, that AGENTS.md claim-labeling rules are followed, and that the DOSSIER.md session entry is present, honest, and append-only — deviations and failed strategies recorded, no prior entries rewritten. Choose `faithful` only when the formalization faithfully captures the claim or transparently declares its deviations, and clear context.review.required_changes to null. Otherwise choose `revise` and set context_updates for context.review.required_changes describing each fidelity gap precisely. Respond with ONLY a JSON decision envelope.
