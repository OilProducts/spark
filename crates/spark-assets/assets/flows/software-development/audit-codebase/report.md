---
type: agent_task
title: Report & Backlog
contracts:
  reads_context:
  - context.findings.confirmed
  - context.findings.rejected
  - context.findings.coverage
  writes_context:
  - context.audit.summary
  - context.audit.backlog
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: report
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Produce the audit verdict and fix backlog from context.findings.confirmed and context.findings.rejected, without changing any files. The summary leads with the overall health assessment and presents each confirmed finding with location, failure scenario, and evidence grade; rejected findings form a short auditable appendix. State every lens that context.findings.coverage records as not completed as missing coverage; never describe the audit as covering it. Then turn the confirmed findings into an ordered fix backlog, most severe first: each entry must be one bounded, self-contained task statement an implementer can execute without reading this run (the defect, the fix, acceptance criteria including a check that would have caught it). Merge findings that one change fixes; leave out findings that need a human decision, noting why. Choose `done` when the report is complete, with context_updates setting context.audit.summary and context.audit.backlog (the ordered task list). Respond with ONLY a JSON decision envelope.
