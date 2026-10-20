---
type: agent_task
title: Synthesize
contracts:
  reads_context:
  - context.arbiter.decision
  - context.review.required_changes
  writes_context:
  - context.findings.summary
  response: status_envelope
routes:
- to: review_findings
---

Synthesize the session into findings.md: the statement history (every version in shared/statement.md with what forced each revision), the verdict — proved, refuted, or open — with the verified evidence behind it, the prover's progress (lemmas proved with Lean names, the final stuck point), the refuter's coverage (verified counterexamples, or exact exhausted scopes and near-misses), and the smallest open case. Read .mathlab/round and .mathlab/max-rounds and note plainly when the session ended on budget rather than resolution. Label every claim per AGENTS.md and state the exact scope of every computation: a proof counts only if its marker passed the kernel gate, a counterexample only if it re-verified. Then append a session entry to DOSSIER.md per its structure and update its status board: statement version attacked, verdict, established results, failed attacks with what they taught, verified data locations, and open subproblems with the stuck lemma first. Dossier entries are append-only. If context.review.required_changes is present, close those gaps. Propose the natural next session in a closing section of findings.md. Respond with ONLY a JSON status envelope whose context_updates set context.findings.summary.
