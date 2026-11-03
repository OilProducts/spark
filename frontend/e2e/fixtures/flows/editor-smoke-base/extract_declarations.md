---
type: agent_task
title: Extract Testable Declarations
contracts:
  response: decision_envelope
routes:
  done:
    to: audit_human_gate
    description: The testable declarations are written.
---

Inspect the repository and write a bounded set of testable declarations.
