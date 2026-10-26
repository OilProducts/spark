---
type: human_gate
title: Human Audit Decision
routes:
- to: audit_rework
  label: Rework Queue State
  extensions:
    loop_restart: false
- to: done
  label: Accept Risk
  extensions:
    loop_restart: false
---

Choose whether to rework or accept risk.
