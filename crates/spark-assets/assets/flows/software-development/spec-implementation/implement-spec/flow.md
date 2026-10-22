---
type: Flow
id: implement_spec_program
title: Implement Spec Program
description: A greenfield software-development program flow. It creates committed spec packages under specs/<slug>/, stores workflow state under .spark/spec-implementation/<slug>/, refreshes .spark/spec-implementation/current/ as a stable runtime alias to the active workspace, surfaces the committed spec package through current/spec/, extracts a requirement ledger, proposes an architecture, plans milestones, dispatches a milestone worker as a child flow, and audits conformance until the specification is covered or the effort is blocked.
schema_version: '1'
inputs:
- key: context.request.spec_path
  label: Spec Path
  type: string
  description: Path to the written spec in the project repository.
  required: true
- key: context.request.spec_slug
  label: Spec Slug
  type: string
  description: Optional human-readable slug for the committed spec package under specs/. If omitted, derive one from the spec filename.
  required: false
- key: context.request.scope_hint
  label: Scope Hint
  type: string
  description: Optional section, milestone, or subset to focus on first.
  required: false
- key: context.request.constraints
  label: Constraints
  type: string[]
  description: Optional implementation constraints or explicit non-goals.
  required: false
- key: context.request.target_paths
  label: Target Paths
  type: string[]
  description: Optional files or directories to focus on first.
  required: false
- key: context.request.validation_command
  label: Validation Command
  type: string
  description: Optional preferred repo-wide validation command. Milestone workers should treat it as the primary gate when it is deterministic and truthful, and use narrower checks only as supplements or when no stronger gate exists.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Implement a written specification in the repository by turning the spec into requirements, architecture, milestones, and validated milestone deliveries until all in-scope requirements are covered or the effort is blocked.
