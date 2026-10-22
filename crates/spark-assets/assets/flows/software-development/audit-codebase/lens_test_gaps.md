---
type: agent_task
title: 'Lens: Test Gaps'
contracts:
  reads_context:
  - context.request.objective
  response: status_envelope
routes:
- to: fan_in
---

Audit the repository read-only through the test-coverage lens: load-bearing behavior with no test exercising its real interface, tests asserting on repository text or mocks instead of observable behavior, validation gates that skip whole surfaces, and regression-prone areas with no regression check. Weight context.request.objective focus areas when present. Do not change any files. For every finding record the untested behavior and its location, the concrete regression that could ship unnoticed, and an evidence grade: strong (deterministic check, direct inspection), medium (clear code-path reading), weak (suspicion). Report only findings you can anchor to specific code. Respond with ONLY a JSON status envelope; put the findings in your response.
