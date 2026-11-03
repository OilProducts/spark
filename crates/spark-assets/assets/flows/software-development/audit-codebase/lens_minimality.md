---
type: agent_task
title: 'Lens: Minimality'
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
    to: lens_minimality
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Audit the repository read-only through the minimality lens, lazy like a senior developer but not negligent: dead or uncalled code, reimplemented stdlib or duplicated in-repo logic, speculative abstractions and future-proofing nothing uses, wrappers and config surfaces with one caller, and complexity that a smaller shape would serve. Weight context.request.objective focus areas when present. Do not change any files. Flag a finding only when the excess creates correctness risk, maintenance burden, or significant complexity — include the concrete delete-or-simplify candidate, and never propose cutting necessary validation, error handling, or safety. Record file and location, the claim, and an evidence grade: strong (direct inspection), medium (code-path reading), weak (suspicion). Choose `done` once this lens review is complete, whether or not it found anything, and put the complete findings (or a statement that there are none) in notes. Choose `blocked` only when you cannot perform this lens review at all. Respond with ONLY a JSON decision envelope.
