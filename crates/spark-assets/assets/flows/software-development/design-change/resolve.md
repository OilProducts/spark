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
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  rereview:
    to: review
    description: Substantive design changes have been made and require independent review again; update the required design summary.
  unfinished:
    to: resolve
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

context.review.findings lists the reviewers' findings on the design at context.design.path. A finding is a required change only when the design is wrong, misses a requirement, or includes a part the request does not need. Resolve each one as your role describes; you may decline any finding whose fix would add work beyond the request, citing the request, a stated non-goal or the project's own instructions as evidence. Decline a finding that does not name what reaches it in real use, citing that. A finding whose problem starts with "Pre-existing:" is a defect the change neither causes nor worsens: list it under Open questions as a follow-up and do not design its fix unless the request asks for it.

When a fix is needed, prefer one that narrows or deletes over one that adds a mechanism such as a lock, guard or new state. When a goal the request does not ask for is what makes a case required, narrow the goal instead. Review must not grow the design beyond the request: before choosing `rereview`, compare the design's scope with the request and cut what the request does not need. Edit the design, and do not change any other file. Add or update a Review section in the design listing each contested or declined finding with its evidence, and each escalation with its answer, so later readers do not raise them again.

Choose `rereview` only if you made substantive changes to the design; otherwise choose `done`. When the `rereview` decision says its target has used all its visits, choose `done` even after substantive changes, and list those changes under Open questions as not reviewed. Record any findings you leave unresolved under Open questions. With either decision, set context_updates for context.design.summary to the problem and approach, what review changed, the contested and declined findings, and the open questions. Respond with ONLY a JSON decision envelope.
