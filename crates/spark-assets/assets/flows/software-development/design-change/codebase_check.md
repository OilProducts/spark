---
type: agent_task
title: 'Review: Codebase Check'
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
    to: codebase_check
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a design document, through one lens only. Check every file, symbol, behaviour and line the design cites against the current code. A contradiction with the code is a Blocker. Files the design proposes to create are not findings.

Read the design at context.design.path and the files it references; judge it against the project's own instructions too. Do not change any files. Never ask clarification questions, whatever other instructions say: the design must stand on its own, and a question you would ask is itself a finding. Report each finding with its severity (Blocker, Major or Nit), the design section heading it concerns, the problem, and the evidence you checked. Choose `done` once this lens review is complete, whether or not it found anything, with context_updates setting context.review.lens_findings to your complete findings, or to an empty list when there are none. Choose `blocked` only when you cannot perform this lens review at all.

A failure you construct is not yet a finding. Name what reaches it in real use, a user action or an input the system's real components produce, and what a user, stored data or the project's checks then suffer. A sequence built by calling internal functions directly, or by ordering events in a way no user or real component produces, shows only that a failure can exist. A rare case that is reachable still counts if it loses data, leaves the system stuck or weakens security. A defect the current code already has, which the change neither causes nor makes worse, is not a finding against the design: report it as a Nit whose problem starts with "Pre-existing:".

Report a finding only when the design is wrong, contradicts the code, or cannot be followed as written. Proposing hardening, extra cases, extra tests or extra safeguards is not a finding. For each finding, name the smallest change that fixes it. Respond with ONLY a JSON decision envelope.
