---
type: agent_task
title: Review Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - context.findings.summary
  writes_context:
  - context.review.required_changes
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  ready:
    to: commit_findings
    description: The review is complete and no blocking issue remains under this node’s stated acceptance criteria; record all required review evidence.
  revise:
    to: synthesize
    description: The completed review requires revision of its subject within the existing scope; record the specific corrections and their evidence.
  unfinished:
    to: review_findings
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a prove/refute session in findings.md, DOSSIER.md, and both branch directories. Check that the verdict matches the mechanically verified artifacts: a proved verdict requires the kernel-checked Lean build in prover/lean whose formal statement faithfully encodes the final statement version in shared/statement.md, and a refuted verdict requires the counterexample to re-verify — run refuter/verify.sh yourself — against that statement as written. Check that every claim is labeled per AGENTS.md and the label is truthful, that computational scopes match what the programs actually cover (read the programs, not the prose), that the statement history in findings.md matches shared/statement.md, and that the DOSSIER.md session entry is honest and append-only — failed attacks recorded, nothing inflated, no prior entries rewritten. Spot-check by re-running at least one verification. Choose `ready` only when the findings and the dossier entry are honest, scoped, and verified, and clear context.review.required_changes to null. Otherwise choose `revise` and set context_updates for context.review.required_changes listing each violation precisely. Respond with ONLY a JSON decision envelope.
