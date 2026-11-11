---
type: agent_task
title: Judge Sol
role: adversarial-reviewer
runtime:
  workdir_from: context.workspace.sol_path
contracts:
  reads_context:
  - context.request.objective
  - context.climb.evidence_dir
  writes_context:
  - context.judge.score
  - context.judge.findings
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: You cannot observe the work well enough to score it; record what is missing in notes.
  scored:
    to: judges_join
    description: The work is scored, the score backed by what you observed.
---

Judge the work in this directory, a checkout of its latest commit, against context.request.objective. Observe it yourself in whatever way the objective calls for, such as running it, taking screenshots or reading its output, and base your score only on what you observed, not on commit messages, comments or other descriptions of the work. When the objective names other work to compare against, observe that too and judge the work against it.

Score the work from 1 to 10, where 10 means its users would gain nothing from further change toward the objective. Record what holds the score down and where you saw it, most damaging first.

This checkout is yours alone, so build and run here as you need; do not commit or push. Save the screenshots or other output your score rests on in the `sol` folder of context.climb.evidence_dir, so people can follow the work's progress round by round. Keep everything else you create, such as scripts and seed data, in this checkout, never in shared places such as /tmp, so the work's author cannot see how you judge it.

Choose `scored` with context_updates setting context.judge.score to your score and context.judge.findings to your findings. Respond with ONLY a JSON decision envelope.
