---
type: agent_task
title: Verify Findings
role: adversarial-reviewer
contracts:
  reads_context:
  - context.findings.list
  - context.findings.scope
  writes_context:
  - context.findings.confirmed
  - context.findings.rejected
  response: status_envelope
routes:
- to: report
  condition: outcome=success
---

Under review: a code review. Treat every finding in context.findings.list as an untrusted claim and actively try to refute it against the repository without modifying any files: re-derive the failure scenario, check whether guards or callers the reviewer missed make it unreachable, and confirm the cited code actually behaves as claimed. Drop findings you refute and findings supported only by weak evidence you cannot strengthen. Do not add new findings. Respond with ONLY a JSON status envelope whose context_updates set context.findings.confirmed (the surviving findings, most severe first, each with its verified failure scenario and final evidence grade) and context.findings.rejected (refuted findings with the refutation, so the reviewer's misses are auditable).
