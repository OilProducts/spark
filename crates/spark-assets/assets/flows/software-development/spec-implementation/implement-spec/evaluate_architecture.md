---
type: agent_task
title: Evaluate Architecture
runtime: {}
contracts:
  reads_context:
  - internal.project_work_dir
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  approve:
    to: plan_milestones
    description: The completed review accepts its subject under every applicable acceptance criterion; write any required review outputs.
  blocked:
    to: blocked_exit
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  rework:
    to: revise_architecture
    description: The architecture review found blocking design defects; write their evidence and required corrections before revision.
  unfinished:
    to: evaluate_architecture
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read <internal.project_work_dir>/spec-implementation/current/spec/architecture.md, <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, <internal.project_work_dir>/spec-implementation/current/spec/source.md, and <internal.project_work_dir>/spec-implementation/current/spec/requirements.json, plus <internal.project_work_dir>/spec-implementation/current/architecture-rework.md if it exists from a previous review round. The durable contract ledger is <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json. Critique the proposed architecture before any milestone planning begins, judging whether it is ready for milestone planning rather than whether it is exhaustive. The architecture's job is to fix the repository topology, implementation boundaries, sequencing, public interface, validation strategy, repository hygiene expectations, and every material spec-versus-architecture resolution; the internal mechanics of a subsystem (state machines, concurrency and lifecycle edge cases, algorithm details) are resolved during milestone work unless they change a boundary, an interface, the sequencing, or spec-visible behavior. Confirm the architecture defines those elements coherently enough to realistically produce a deliverable repository from the current spec and repository context. Confirm that every material spec-versus-architecture resolution is explicitly captured in <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json, and that each decision links requirement_ids, spec_refs, architecture_refs, behavioral_contract, implementation_contract, and validation_expectations coherently. Classify each finding as blocking or deferrable. A finding is blocking only if milestone planning built on the current documents would be wrong: the architecture or decisions contradict the spec or silently narrow, rename, or reinterpret product behavior without an explicit approved decision; boundaries, interfaces, or sequencing are missing, contradictory, or infeasible; architecture.md and contract-decisions.json disagree on an architecture-level matter; or a requirement has no credible home. Every other finding is deferrable: a mechanism detail, edge case, or validation refinement that a milestone can resolve within the stated boundaries. If a previous architecture-rework.md exists, first check each of its items against the revision and state which are resolved; do not reopen resolved items, and hold any new finding to the same blocking bar rather than reviewing at a finer level of detail than the previous round. When there are deferrable findings, write them to <internal.project_work_dir>/spec-implementation/current/architecture-open-items.md, each naming the affected area, requirement_ids or decision ids where known, and what the milestone must settle, so milestone planning can assign them; otherwise remove any stale copy of that file. If there are no blocking findings, choose `approve`. If there are blocking findings, write <internal.project_work_dir>/spec-implementation/current/architecture-rework.md listing only the blocking findings, each with its evidence, the required correction, and how the correction can be validated, followed by the status of the previous round's items, and choose `rework`. An unsound architecture or decisions that are materially inconsistent with the spec are blocking findings for `rework`, however extensive the correction. Ambiguity or under-specification in the spec is not a reason to stop: require the revision to choose the most defensible interpretation consistent with the spec, record it as a new decision in <internal.project_work_dir>/spec-implementation/current/spec/contract-decisions.json using the existing decision schema, and design on that basis. Choose `blocked` only when progress requires something that no action within this repository and machine can supply, such as credentials, access to a paid or third-party account, or physical hardware, and no workaround, substitute, or deferral can keep the work moving. Respond with ONLY a JSON decision envelope.
