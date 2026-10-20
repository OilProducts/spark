---
type: agent_task
title: Frame Problem
contracts:
  reads_context:
  - context.request.conjecture
  - context.request.notes
  - context.request.max_rounds
  writes_context:
  - context.statement.current
  response: status_envelope
routes:
- to: fan_out
---

Read DOSSIER.md first per AGENTS.md dossier discipline: it records prior sessions' results, failed attacks, and the current statement version. Frame this session without attempting proofs or experiments yet. Write shared/statement.md containing Statement v1: the conjecture from context.request.conjecture restated precisely, with every definition, quantifier, and range pinned so that a proof attempt and a counterexample search attack the same claim; fold in context.request.notes when present, and if the dossier already carries a repaired statement version for this problem, adopt that as the starting statement and say so. Initialize shared/exchange.md with a Round 0 section: what is known, the most promising proof avenues, the most promising counterexample families, and any dossier-recorded failed attacks that must not be repeated blind. Write the round budget to .mathlab/max-rounds: the integer value of context.request.max_rounds, or 3 when it is unset. Respond with ONLY a JSON status envelope whose context_updates set context.statement.current.
