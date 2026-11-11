---
type: agent_task
title: Council Opus
role: adversarial-reviewer
contracts:
  reads_context:
  - context.climb.objective
  - context.climb.rubric
  - context.climb.references
  - context.climb.evidence_dir
  - context.climb.baseline_evidence_dir
  - context.workspace.path
  - context.workspace.base_commit
  writes_context:
  - context.council.scores
  - context.council.reference_scores
  - context.council.findings
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: You cannot observe the evidence needed to score, such as images you cannot open; record what is missing in notes.
  scored:
    to: council_join
    description: Every subject is scored in every rubric category, each score backed by evidence you observed.
---

Score the work against context.climb.objective, giving each category in context.climb.rubric a score from 1 to 10, where 10 means its users would gain nothing from further change in that category. The work is HEAD of the git worktree at context.workspace.path. context.climb.evidence_dir holds this round's evidence of it; when that directory is empty, inspect the worktree and run what you need instead.

Score two other kinds of subject in the same categories, so a score means the same thing whichever subject receives it: the work as it stood at context.workspace.base_commit, whose evidence is in context.climb.baseline_evidence_dir, as `baseline`; and each item in context.climb.references, keyed by the item as written.

Base every score on what you observed yourself, such as images you opened or output you read, not on commit messages, comments or other descriptions of the work. For each category where the work scores below 10, record what holds it down and where you saw it. Judge only what the objective asks for. Do not edit files, stage or commit.

Choose `scored` with context_updates setting context.council.scores to an object mapping each rubric category, exactly as written, to the work's score; context.council.reference_scores to an object mapping `baseline` and each reference to its own object of category scores; and context.council.findings to your findings, most damaging first. Respond with ONLY a JSON decision envelope.
