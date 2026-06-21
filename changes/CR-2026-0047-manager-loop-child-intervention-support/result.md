---
id: CR-2026-0047-manager-loop-child-intervention-support
title: Manager-Loop Child Intervention Support
status: completed
type: feature
changelog: public
---

## Summary
Implemented manager-loop and human/API intervention support for active child runs on Spark's local execution paths. Manager-loop `steer` now builds child intervention requests from child failure context, records delivered or rejected intervention results without failing the manager node, updates the documented child intervention context fields, emits intervention events, and writes intervention artifacts.

Backend steering is implemented as an optional capability: Codex app-server runs can steer an active turn, unified-agent runs can queue steering on the active session, provider-router runs forward to the active concrete backend, and unsupported or inactive paths return structured rejected results.

## Validation
- `uv run pytest -q`
- Result: 1906 passed, 26 skipped in 28.09s.

## Shipped Changes
- Added `ChildInterventionRequest` and `ChildInterventionResult` runtime dataclasses plus optional intervention requester wiring through handler runtime, handler runners, pipeline runtime wrappers, and local container execution.
- Extended the local container worker protocol with `child_intervention_request` and `child_intervention_result` payload conversion and round-trip handling.
- Added manager-loop intervention delivery, rejection handling, context updates, `ChildInterventionRequested` event emission, and enriched `manager_interventions.jsonl` artifact output.
- Added Codex app-server `turn/steer` client support and active-turn tracking in `CodexAppServerBackend`.
- Added active-session steering for `UnifiedAgentBackend` and active-backend forwarding for `ProviderRouterBackend`.
- Added `POST /attractor/pipelines/{pipeline_id}/steer` behavior via the existing router prefix, including empty-message validation, default active-child targeting from the parent checkpoint, explicit target overrides, rejected inactive-target results, and `HumanInterventionRequested` event publication.
- Added tests covering manager-loop intervention behavior, backend steering paths, local container protocol forwarding, Codex app-server request shape and interleaved responses, and the pipeline steer API endpoint.
