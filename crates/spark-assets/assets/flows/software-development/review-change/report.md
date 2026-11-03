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

Write the final review verdict for the change described by context.findings.scope, based only on context.findings.confirmed and context.findings.rejected. Lead with the overall assessment: whether the change is safe to integrate, blocked by specific findings, or acceptable with nonblocking cleanup. Present each confirmed finding with its location, concrete failure scenario, and evidence grade; keep rejected findings as a short auditable appendix. Do not soften confirmed findings or resurrect rejected ones. Choose `done` when the report is complete, whatever its verdict, with context_updates setting context.review.verdict (integrate, blocked, or cleanup) and context.review.summary (the complete report); a blocked verdict is report data, not the `blocked` decision. Respond with ONLY a JSON decision envelope.
