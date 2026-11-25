---
type: Flow
id: mission_retrospective
title: Mission Retrospective
description: Review how a Spark mission ran from its recorded state, conversation, and child runs, adversarially verify the assessment, and recommend targeted improvements.
schema_version: '2'
inputs:
- key: context.request.mission_id
  label: Mission ID
  type: string
  description: The mission to review.
  required: true
- key: context.request.focus
  label: Focus
  type: string
  multiline: true
  description: Optional questions, outcomes, or concerns to weight the retrospective toward.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.2-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Review how a Spark mission ran from its recorded state, conversation, and child runs, adversarially verify the assessment, and recommend targeted improvements.
