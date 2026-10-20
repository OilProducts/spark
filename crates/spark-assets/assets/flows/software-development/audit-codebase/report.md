---
type: agent_task
title: Report & Backlog
contracts:
  reads_context:
  - context.findings.confirmed
  - context.findings.rejected
  writes_context:
  - context.audit.summary
  - context.audit.backlog
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

Produce the audit verdict and fix backlog from context.findings.confirmed and context.findings.rejected, without changing any files. The summary leads with the overall health assessment and presents each confirmed finding with location, failure scenario, and evidence grade; rejected findings form a short auditable appendix. Then turn the confirmed findings into an ordered fix backlog, most severe first: each entry must be one bounded, self-contained task statement an implementer can execute without reading this run (the defect, the fix, acceptance criteria including a check that would have caught it). Merge findings that one change fixes; leave out findings that need a human decision, noting why. Respond with ONLY a JSON status envelope whose context_updates set context.audit.summary and context.audit.backlog (the ordered task list).
