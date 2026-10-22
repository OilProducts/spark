---
type: agent_task
title: Review Findings
role: adversarial-reviewer
runtime:
  error_policy: continue
contracts:
  reads_context:
  - context.findings.summary
  - context.experiments.summary
  writes_context:
  - context.review.required_changes
  response: status_envelope
routes:
- to: commit_findings
- to: experiment
  label: Fix Evidence
  condition: outcome=fail && preferred_label=Experiment
  extensions:
    loop_restart: false
- to: synthesize
  label: Fix Findings
  condition: outcome=fail && preferred_label=Synthesize
  extensions:
    loop_restart: false
---

Under review: experimental mathematics. Treat findings.md, DOSSIER.md, and experiments/ as untrusted claims. Check that every claim is labeled per AGENTS.md and the label is truthful; that computational scopes are stated exactly and match what the programs actually cover (read the programs, not the prose); that certificates exist and experiments/verify.sh genuinely re-checks them rather than rubber-stamping; and that nothing presented as evidence is unreproducible. Check the DOSSIER.md session entry: it must be present, honest, and append-only — failed attacks recorded, nothing inflated, no prior entries rewritten. Spot-check by re-running at least one experiment. Return outcome success only when the findings are honest, scoped, and verified. On failure, set context_updates for context.review.required_changes listing each violation precisely, and route your outcome preferred_label to the node that owns the fix: Experiment for evidence problems, Synthesize for labeling or scope problems. On success, clear context.review.required_changes to null. Respond with ONLY a JSON status envelope.
