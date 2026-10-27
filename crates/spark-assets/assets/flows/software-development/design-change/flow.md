---
type: Flow
id: design_change
title: Design Change
description: Draft a design document, review it with isolated single-lens reviewers, resolve the findings in the drafter's session, and re-review once if anything changed. The run ends with a reviewed design at a known path.
schema_version: '1'
inputs:
- key: context.request.objective
  label: Objective
  type: string
  description: The change wanted, as the requester stated it, with the relevant decisions, constraints and prior attempts the requester knows of.
  required: true
- key: context.request.target_paths
  label: Target Paths
  type: string[]
  description: ''
  required: false
- key: context.request.constraints
  label: Constraints
  type: string[]
  description: ''
  required: false
- key: context.request.design_path
  label: Design Path
  type: string
  description: Where to write the design. Defaults to where the project's instructions keep designs, else docs/designs/<slug>.md.
  required: false
- key: context.request.artifact_path
  label: Artifact Path
  type: string
  description: An existing design to review instead of drafting one.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Draft a design document, review it with isolated single-lens reviewers, resolve the findings in the drafter's session, and re-review once if anything changed. The run ends with a reviewed design at a known path.
