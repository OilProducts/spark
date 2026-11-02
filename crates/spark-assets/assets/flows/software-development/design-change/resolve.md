---
type: agent_task
title: Resolve Findings
role: author
runtime:
  session: designer
contracts:
  reads_context:
  - context.design.path
  - context.review.findings
  writes_context:
  - context.review.contested
  - context.design.summary
  response: status_envelope
routes:
- to: done
  label: Done
- to: review
  label: Re-review
---

context.review.findings lists the reviewers' findings on the design at context.design.path. A finding is a required change only when the design is wrong or misses a requirement. Resolve each one as your role describes; you may decline any finding whose fix would add work beyond the request, citing the request, a stated non-goal or the project's own instructions as evidence. Edit the design, and do not change any other file. Add or update a Review section in the design listing each contested or declined finding with its evidence, and each escalation with its answer, so later readers do not raise them again.

Answer Re-review only if you made substantive changes to the design; otherwise answer Done. If Re-review is unavailable, record any unresolved findings under Open questions and answer Done. Respond with ONLY a JSON status envelope whose context_updates set context.design.summary to the problem and approach, what review changed, the contested and declined findings, and the open questions.
