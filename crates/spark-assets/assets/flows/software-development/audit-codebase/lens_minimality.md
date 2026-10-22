---
type: agent_task
title: 'Lens: Minimality'
contracts:
  reads_context:
  - context.request.objective
  response: status_envelope
routes:
- to: fan_in
---

Audit the repository read-only through the minimality lens, lazy like a senior developer but not negligent: dead or uncalled code, reimplemented stdlib or duplicated in-repo logic, speculative abstractions and future-proofing nothing uses, wrappers and config surfaces with one caller, and complexity that a smaller shape would serve. Weight context.request.objective focus areas when present. Do not change any files. Flag a finding only when the excess creates correctness risk, maintenance burden, or significant complexity — include the concrete delete-or-simplify candidate, and never propose cutting necessary validation, error handling, or safety. Record file and location, the claim, and an evidence grade: strong (direct inspection), medium (code-path reading), weak (suspicion). Respond with ONLY a JSON status envelope; put the findings in your response.
