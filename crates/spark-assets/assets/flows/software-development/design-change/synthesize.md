---
type: agent_task
title: Synthesize Findings
contracts:
  reads_context:
  - parallel.results
  writes_context:
  - context.review.findings
  response: status_envelope
routes:
- to: resolve
  condition: outcome=success
---

In parallel.results, each reviewer branch's id names its lens, and context.review.lens_findings in that branch's context holds the reviewer's complete findings; the fan_in entry holds no review. A reviewer branch that failed or left context.review.lens_findings unset is itself a finding: report it as a Blocker under that lens. Merge the reviewers' findings into one de-duplicated list without changing any files. Each finding keeps the lens or lenses that raised it, its severity (Blocker, Major or Nit; when merged findings disagree, keep the highest), the design section heading it concerns, the problem, and the reviewer's evidence. Do not judge the findings again, drop any, or add new ones. Respond with ONLY a JSON status envelope whose context_updates set context.review.findings to the merged list, most severe first, or to an empty list when there are none.
