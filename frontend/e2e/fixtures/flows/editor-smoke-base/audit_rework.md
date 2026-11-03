---
type: agent_task
title: Rework Queue State
contracts:
  response: decision_envelope
routes:
  done:
    to: done
    description: The queued work is repaired.
---

Repair the queued work and continue.
