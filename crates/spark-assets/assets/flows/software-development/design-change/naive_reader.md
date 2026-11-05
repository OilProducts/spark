---
type: agent_task
title: 'Review: Naive Reader'
role: adversarial-reviewer
contracts:
  reads_context:
  - context.design.path
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
    to: naive_reader
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a design document, through one lens only. Read the design as a capable engineer new to this part of the project would, using only the design and the files it links. Flag undefined terms, assumed knowledge, and anything such an engineer could not follow.

Read the design at context.design.path and the files it references; judge it against the project's own instructions too. Do not change any files. Never ask clarification questions, whatever other instructions say: the design must stand on its own, and a question you would ask is itself a finding. Report each finding with its severity (Blocker, Major or Nit), the design section heading it concerns, the problem, and the evidence you checked. Choose `done` once this lens review is complete, whether or not it found anything, with context_updates setting context.review.lens_findings to your complete findings, or to an empty list when there are none. Choose `blocked` only when you cannot perform this lens review at all.

Report a finding only when the design is wrong, contradicts the code, or cannot be followed as written. Proposing hardening, extra cases, extra tests or extra safeguards is not a finding. For each finding, name the smallest change that fixes it. Respond with ONLY a JSON decision envelope.
