---
type: agent_task
title: 'Review: Zoom Out'
role: adversarial-reviewer
contracts:
  reads_context:
  - context.design.path
  - context.request.objective
  - context.request.constraints
  writes_context:
  - context.review.lens_findings
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: fan_in
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: zoom_out
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a design document, through one lens only. Zoom out to the root problem and simplicity. Does each part of the design need to exist? Every special case the design handles must say what reaches it; one that does not is a part the request does not need. Does it reuse existing code? Search the repository for an existing mechanism before accepting anything new. Flag a design that adds or keeps a parallel implementation of something the codebase already does elsewhere, including one patched to match the other: it should leave one implementation. Is the altitude right: no implementation detail crowding the approach, no hand-waving in the plan? Does any part lead to a dead end?

Read the request in context.request.objective and context.request.constraints, and the design at context.design.path and the files it references. Judge the design against the request and the project's own instructions. Do not change any files. Never ask clarification questions, whatever other instructions say: the design must stand on its own, and a question you would ask is itself a finding. Report each finding with its severity (Blocker, Major or Nit), the design section heading it concerns, the problem, and the evidence you checked. Choose `done` once this lens review is complete, whether or not it found anything, with context_updates setting context.review.lens_findings to your complete findings, or to an empty list when there are none. Choose `blocked` only when you cannot perform this lens review at all.

Report a finding only when the design is wrong, contradicts the code, misses something the request requires, cannot be followed as written, or includes a part the request does not need. Proposing extra hardening, cases, tests or safeguards is not a finding; proposing to remove them is. For each finding, name the smallest change that fixes it, which may be deleting that part. Respond with ONLY a JSON decision envelope.
