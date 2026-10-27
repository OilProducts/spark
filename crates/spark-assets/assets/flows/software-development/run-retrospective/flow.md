---
type: Flow
id: run_retrospective
title: Run Retrospective
description: Diagnose why a Spark run went wrong from its recorded artifacts, adversarially verify the diagnosis, and recommend a fix. The run ends with the verified diagnosis and recommended fix.
schema_version: '1'
inputs:
- key: context.request.run_id
  label: Run ID
  type: string
  description: The run to analyze.
  required: true
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Diagnose why a Spark run went wrong from its recorded artifacts, adversarially verify the diagnosis, and recommend a fix. The run ends with the verified diagnosis and recommended fix.
