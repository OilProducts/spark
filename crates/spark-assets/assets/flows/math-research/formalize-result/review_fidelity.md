---
type: agent_task
title: Review Fidelity
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.request.statement
  - context.formalization.theorems
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: commit_result
- to: formalize
  label: Fix Fidelity
  condition: outcome=fail
  extensions:
    loop_restart: false
---

Under review: a Lean formalization. The build already passed the kernel gate; your job is the part the kernel cannot check: does the formal statement faithfully encode context.request.statement? Read the theorems named in .mathlab/theorems.txt inside lean/, and for each one check quantifier structure, hypotheses, definitional choices, and edge conditions against the informal claim — a proved theorem about the wrong statement is a violation, not a result. Run (cd lean && lake env lean) probes or #check/#print inspections as needed. Verify any stated deviations in .mathlab/notes.md are honest and prominently declared, that AGENTS.md claim-labeling rules are followed, and that the DOSSIER.md session entry is present, honest, and append-only — deviations and failed strategies recorded, no prior entries rewritten. Return outcome success only when the formalization faithfully captures the claim or transparently declares its deviations. On failure, set context_updates for context.review.required_changes describing each fidelity gap precisely. On success, clear context.review.required_changes to null. Respond with ONLY a JSON status envelope.
