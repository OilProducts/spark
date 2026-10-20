---
type: agent_task
title: 'Lens: Safety & Security'
contracts:
  reads_context:
  - context.request.objective
  response: status_envelope
routes:
- to: fan_in
---

Audit the repository read-only through the safety and security lens: injection and path-traversal surfaces, secrets handling, data-loss hazards, destructive operations without guards, trust boundaries crossed without validation, and unsafe defaults. Weight context.request.objective focus areas when present. Do not change any files. For every finding record the file and location, the defect claim, the concrete scenario in which it fails, and an evidence grade: strong (deterministic check, direct inspection), medium (clear code-path reading), weak (suspicion). Report only findings you can anchor to specific code. Respond with ONLY a JSON status envelope; put the findings in your response.
