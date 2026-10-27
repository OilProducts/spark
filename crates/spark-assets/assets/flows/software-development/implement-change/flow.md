---
type: Flow
id: implement_change
title: Implement Change
description: Implement one bounded repository change in an isolated worktree, gatekeep it against its contract, and commit the validated result.
schema_version: '1'
inputs:
- key: context.request.objective
  label: Objective
  type: string
  description: Inline task objective. Provide this or artifact_path.
  required: false
- key: context.request.artifact_path
  label: Artifact Path
  type: string
  description: Repository-relative task artifact. Provide this or objective.
  required: false
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
- key: context.request.acceptance_criteria
  label: Acceptance Criteria
  type: string[]
  description: ''
  required: false
- key: context.request.validation_command
  label: Validation Command
  type: string
  description: ''
  required: false
- key: context.request.base_ref
  label: Base Ref
  type: string
  description: ''
  required: false
defaults:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Implement one bounded repository change in an isolated worktree, gatekeep it against its contract, and commit the validated result.
