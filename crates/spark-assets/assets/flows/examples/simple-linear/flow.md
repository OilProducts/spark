---
type: Flow
id: simple_linear
title: Simple Linear Workflow
description: A minimal starter flow that plans a small change, executes it, and summarizes the outcome.
schema_version: '1'
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Inspect the repository, make one targeted improvement, and summarize the result.
