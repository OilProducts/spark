---
type: Flow
id: merge_change
title: Merge Change
description: Integrate a completed Spark branch into a clean local target using an explicit merge commit, validating only when conflicts require resolution.
execution_lock:
  scope: project
  key: software-development-integration
  conflict_policy: queue
schema_version: '1'
inputs:
- key: context.request.source_ref
  label: Source Ref
  type: string
  description: Branch or commit to integrate into the current target checkout.
  required: true
- key: context.request.validation_command
  label: Validation Command
  type: string
  description: ''
  required: false
defaults:
  fidelity: summary:high
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Integrate a completed Spark branch into a clean local target using an explicit merge commit, validating only when conflicts require resolution.
