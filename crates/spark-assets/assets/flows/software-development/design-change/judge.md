---
type: agent_task
title: Judge Proposals
contracts:
  reads_context:
  - parallel.results
  - context.request.objective
  writes_context:
  - context.design.task
  - context.design.rationale
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

Judge the competing proposals in parallel.results for context.request.objective. Treat each proposal's claims as untrusted: spot-check its repository assertions read-only, and weight evidence over rhetoric. Score on: truthfulness to the actual codebase, minimality (does each proposed code path need to exist), risk to existing behavior, and completeness of acceptance criteria. Prefer the smallest proposal whose risks are honestly priced; graft clearly superior ideas from the losers into the winner when they compose without expanding scope. Respond with ONLY a JSON status envelope whose context_updates set context.design.task (one decision-complete, self-contained task statement an implementer can execute without reading this run — objective, concrete changes, acceptance criteria, constraints, validation command) and context.design.rationale (the comparative judgment: why the winner won and what was grafted or rejected, so the human can audit the choice).
