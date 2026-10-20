---
type: agent_task
title: Repository Integrity Audit
runtime: {}
contracts:
  response: status_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: blocked_exit
  label: Blocked
  condition: outcome=fail && preferred_label=Blocked
  extensions:
    loop_restart: false
- to: done
  label: Complete
  condition: outcome=success
  extensions:
    loop_restart: false
- to: plan_cleanup_milestone
  label: Cleanup
  condition: outcome=fail && preferred_label=Extend
  extensions:
    loop_restart: false
- to: plan_cleanup_milestone
  label: Cleanup Partial
  condition: outcome=partial_success
  extensions:
    loop_restart: false
- to: blocked_exit
  label: Blocked Partial
  condition: outcome=partial_success && preferred_label=Blocked
  weight: 1
  extensions:
    loop_restart: false
---

Read .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, .spark/spec-implementation/current/spec/architecture.md, and .spark/spec-implementation/current/spec/contract-decisions.json, and the current repository state as needed. Decide whether the repository is now honestly deliverable, not merely internally coherent. Check that the repository can be built, run, imported, and validated from committed metadata, manifests, bootstrap files, docs, public exports, and the approved contract decisions when applicable; that tests are behavioral rather than brittle source-text or path-hack checks; that tracked local, generated, cache, IDE, or Spark workflow artifacts are not part of the deliverable; that docs, exports, manifests, and ignore rules stay coherent; and that no obvious duplicate validators or mixed-responsibility god files remain as the final state. Success requires a clear source of truth for delivered surfaces, no unjustified wrapper-only duplicate layers, essential repo metadata and docs for the technologies in use, and no bound contract decision left satisfied only through weaker internal or metadata-only paths. If the repository is deliverable, respond with ONLY a JSON status envelope with outcome success. If further cleanup/refactor work can close the remaining gaps, write .spark/spec-implementation/current/repository-integrity-gaps.md with the missing work. The cleanup ledger is .spark/spec-implementation/current/repository-integrity-gaps.md. Return outcome fail with preferred_label Extend. Large or redesign-level gaps are still Extend work: describe them in the cleanup ledger so they can be planned as one or more cleanup/refactor milestones. Return outcome fail with preferred_label Blocked only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Respond with ONLY a JSON status envelope.
