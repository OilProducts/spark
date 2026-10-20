---
type: agent_task
title: Report
contracts:
  reads_context:
  - context.findings.scope
  - context.findings.confirmed
  - context.findings.rejected
  writes_context:
  - context.review.verdict
  - context.review.summary
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

Write the final review verdict for the change described by context.findings.scope, based only on context.findings.confirmed and context.findings.rejected. Lead with the overall assessment: whether the change is safe to integrate, blocked by specific findings, or acceptable with nonblocking cleanup. Present each confirmed finding with its location, concrete failure scenario, and evidence grade; keep rejected findings as a short auditable appendix. Do not soften confirmed findings or resurrect rejected ones. Respond with ONLY a JSON status envelope whose context_updates set context.review.verdict (integrate, blocked, or cleanup) and context.review.summary (the complete report).
