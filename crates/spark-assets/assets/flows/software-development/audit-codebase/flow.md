---
type: Flow
id: audit_codebase
title: Audit Codebase
description: Sweep the repository through parallel audit lenses and adversarially verify the findings. The run ends with the verified findings and an ordered fix backlog.
schema_version: '1'
inputs:
- key: context.request.objective
  label: Audit Focus
  type: string
  description: Optional focus areas or subsystems to weight the audit toward.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Sweep the repository through parallel audit lenses and adversarially verify the findings. The run ends with the verified findings and an ordered fix backlog.
