---
type: agent_task
title: Verify Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - parallel.results
  writes_context:
  - context.findings.confirmed
  - context.findings.rejected
  response: status_envelope
routes:
- to: report
  condition: outcome=success
---

Under review: a codebase audit. The lens findings are in parallel.results; treat every finding as an untrusted claim. Deduplicate findings that describe one underlying defect, then actively try to refute each against the repository without changing any files: re-derive the failure scenario, check for guards or callers the lens missed, and confirm the cited code behaves as claimed. Drop findings you refute and findings supported only by weak evidence you cannot strengthen. Do not add new findings. Respond with ONLY a JSON status envelope whose context_updates set context.findings.confirmed (surviving findings, most severe first, each with location, verified failure scenario, and final evidence grade) and context.findings.rejected (refuted findings with the refutation, so misses stay auditable).
