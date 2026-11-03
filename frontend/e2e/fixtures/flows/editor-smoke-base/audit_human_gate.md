---
type: human_gate
title: Human Audit Decision
routes:
  accept_risk:
    to: done
    description: The remaining risk is accepted.
    label: Accept Risk
  rework:
    to: audit_rework
    description: The queued work needs rework.
    label: Rework Queue State
---

Choose whether to rework or accept risk.
