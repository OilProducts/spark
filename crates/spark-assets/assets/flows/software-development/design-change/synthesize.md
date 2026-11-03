---
type: agent_task
title: Synthesize Findings
contracts:
  reads_context:
  - parallel.results
  writes_context:
  - context.review.findings
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: resolve
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: synthesize
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

parallel.results holds one entry per reviewer branch in declared order; each entry's id names its lens, and for a `completed` entry context.review.lens_findings in that entry's context holds the reviewer's complete findings. A reviewer branch whose status is not `completed` (`error`, `terminal_result` or `canceled`), or that left context.review.lens_findings unset, did not review the design and is itself a finding: report it as a Blocker under that lens, citing its status and any recorded `error`, and never treat it as a lens with no findings. Merge the reviewers' findings into one de-duplicated list without changing any files. Each finding keeps the lens or lenses that raised it, its severity (Blocker, Major or Nit; when merged findings disagree, keep the highest), the design section heading it concerns, the problem, and the reviewer's evidence. Do not judge the findings again, drop any, or add new ones. Choose `done` when the merge is complete, with context_updates setting context.review.findings to the merged list, most severe first, or to an empty list when there are none. Respond with ONLY a JSON decision envelope.
