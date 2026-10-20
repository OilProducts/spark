---
type: agent_task
title: Resolve Conflicts
contracts:
  reads_context:
  - context.integration.source_commit
  - context.integration.target_commit
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

The current working directory is an integration worktree with an in-progress merge (--no-ff --no-commit). If git reports no conflicted paths, succeed without changing anything. Otherwise resolve only conflicts whose behavior-preserving resolution is unambiguous and stage the resolved files; when any conflict requires judgment about intended behavior, fail and leave the worktree inspectable. Do not commit, validate, move branches, or clean up; the parent flow performs those operations. Respond with ONLY a JSON status envelope.
