---
type: agent_task
title: 'Lens: Safety & Security'
contracts:
  reads_context:
  - context.request.objective
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: fan_in
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: lens_safety
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Audit the repository read-only through the safety and security lens: injection and path-traversal surfaces, secrets handling, data-loss hazards, destructive operations without guards, trust boundaries crossed without validation, and unsafe defaults. Weight context.request.objective focus areas when present. Do not change any files. For every finding record the file and location, the defect claim, the concrete scenario in which it fails, and an evidence grade: strong (deterministic check, direct inspection), medium (clear code-path reading), weak (suspicion). Report only findings you can anchor to specific code. Choose `done` once this lens review is complete, whether or not it found anything, and put the complete findings (or a statement that there are none) in notes. Choose `blocked` only when you cannot perform this lens review at all. Respond with ONLY a JSON decision envelope.
