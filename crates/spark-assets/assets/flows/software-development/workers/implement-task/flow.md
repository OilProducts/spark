---
type: Flow
id: software_development_implement_task
title: Implement Task
description: Hidden reusable software-development worker.
schema_version: '1'
defaults:
  fidelity: summary:high
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: false
    worker: true
---

Implement, test, self-review, and commit the bounded task inside the current isolated worktree.
