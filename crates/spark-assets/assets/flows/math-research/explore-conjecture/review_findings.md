---
type: agent_task
title: Review Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - context.findings.summary
  - context.experiments.summary
  writes_context:
  - context.review.required_changes
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  experiment:
    to: experiment
    description: The findings review found an evidence or reproducibility defect owned by the experiment step; record the required corrections.
  ready:
    to: commit_findings
    description: The review is complete and no blocking issue remains under this node’s stated acceptance criteria; record all required review evidence.
  synthesize:
    to: synthesize
    description: The findings review found a labeling, scope or report defect owned by synthesis; record the required corrections.
  unfinished:
    to: review_findings
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: experimental mathematics in findings.md, DOSSIER.md, and experiments/. Check that every claim is labeled per AGENTS.md and the label is truthful; that computational scopes are stated exactly and match what the programs actually cover (read the programs, not the prose); that certificates exist and experiments/verify.sh genuinely re-checks them rather than rubber-stamping; and that nothing presented as evidence is unreproducible. Check the DOSSIER.md session entry: it must be present, honest, and append-only — failed attacks recorded, nothing inflated, no prior entries rewritten. Spot-check by re-running at least one experiment. Choose `ready` only when the findings are honest, scoped, and verified, and clear context.review.required_changes to null. Otherwise set context_updates for context.review.required_changes listing each violation precisely, and choose the decision for the node that owns the fix: `experiment` for evidence problems, `synthesize` for labeling or scope problems. Respond with ONLY a JSON decision envelope.
