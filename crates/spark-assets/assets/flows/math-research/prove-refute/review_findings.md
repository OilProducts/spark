---
type: agent_task
title: Review Findings
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.findings.summary
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: commit_findings
- to: synthesize
  label: Fix Findings
  condition: outcome=fail
  extensions:
    loop_restart: false
---

Under review: a prove/refute session. Treat findings.md, DOSSIER.md, and both branch directories as untrusted claims. Check that the verdict matches the mechanically verified artifacts: a proved verdict requires the kernel-checked Lean build in prover/lean whose formal statement faithfully encodes the final statement version in shared/statement.md, and a refuted verdict requires the counterexample to re-verify — run refuter/verify.sh yourself — against that statement as written. Check that every claim is labeled per AGENTS.md and the label is truthful, that computational scopes match what the programs actually cover (read the programs, not the prose), that the statement history in findings.md matches shared/statement.md, and that the DOSSIER.md session entry is honest and append-only — failed attacks recorded, nothing inflated, no prior entries rewritten. Spot-check by re-running at least one verification. Return outcome success only when the findings and the dossier entry are honest, scoped, and verified. On failure, set context_updates for context.review.required_changes listing each violation precisely. On success, clear context.review.required_changes to null. Respond with ONLY a JSON status envelope.
