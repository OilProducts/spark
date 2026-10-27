---
type: agent_task
title: 'Review: Zoom Out'
role: adversarial-reviewer
contracts:
  reads_context:
  - context.design.path
  writes_context:
  - context.review.lens_findings
  response: status_envelope
routes:
- to: fan_in
---

Under review: a design document, through one lens only. Zoom out to the root problem and simplicity. Does each part of the design need to exist? Does it reuse existing code? Search the repository for an existing mechanism before accepting anything new. Is the altitude right: no implementation detail crowding the approach, no hand-waving in the plan? Does any part lead to a dead end?

Read the design at context.design.path and the files it references; judge it against the project's own instructions too. Do not change any files. Never ask clarification questions, whatever other instructions say: the design must stand on its own, and a question you would ask is itself a finding. Report each finding with its severity (Blocker, Major or Nit), the design section heading it concerns, the problem, and the evidence you checked. Respond with ONLY a JSON status envelope whose context_updates set context.review.lens_findings to your complete findings, or to an empty list when there are none.
