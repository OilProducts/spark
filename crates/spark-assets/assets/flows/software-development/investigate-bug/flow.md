---
type: Flow
id: investigate_bug
title: Investigate Bug
description: Investigate a reported defect, adversarially verify the diagnosis, propose a repair plan, and adversarially verify the plan. The run ends with the verified diagnosis and plan.
schema_version: '1'
inputs:
- key: context.request.objective
  label: Bug Report
  type: string
  description: The observed defect, including reproduction hints and expected behavior when known.
  required: true
- key: context.request.target_paths
  label: Target Paths
  type: string[]
  description: ''
  required: false
- key: context.request.validation_command
  label: Validation Command
  type: string
  description: ''
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Investigate a reported defect, adversarially verify the diagnosis, propose a repair plan, and adversarially verify the plan. The run ends with the verified diagnosis and plan.
