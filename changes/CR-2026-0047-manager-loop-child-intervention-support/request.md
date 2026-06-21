# Manager-Loop Child Intervention Support

## Summary
Add manager-loop and human/API intervention support for active child runs, limited to Spark’s supported execution paths: in-process local execution and local container execution. The implementation will not add remote-worker functionality, and backend steering remains an optional capability.

## Key Changes
- Add `ChildInterventionRequest` / `ChildInterventionResult` dataclasses beside existing child-run types.
- Extend `HandlerRuntime`, `HandlerRunner`, and `ContainerizedHandlerRunner` with an optional `child_intervention_requester`.
- Extend the local container worker protocol with:
  - request: `child_intervention_request`
  - response: `child_intervention_result`
- Manager loop behavior:
  - `manager.actions=steer` attempts intervention only when the child reports failure context.
  - If the child/backend is not actively steerable, record a rejected result rather than failing the manager node.
  - Write context fields for `context.stack.child.intervention`, `intervention_status`, `intervention_delivery_mode`, and `intervention_reason`.
  - Emit `ChildInterventionRequested` and append enriched `manager_interventions.jsonl`.

## Backend and API Behavior
- Add `CodexAppServerClient.steer_turn(thread_id, turn_id, message)` and tests for its JSON-RPC request shape.
- `CodexAppServerBackend` tracks the active app-server turn using `run_turn(..., on_turn_started=...)`; intervention calls `steer_turn` while a turn is active.
- `UnifiedAgentBackend` tracks the active `Session`; intervention calls `session.steer(message)`.
- `ProviderRouterBackend` forwards intervention to the currently active concrete backend when present.
- Unsupported or inactive paths return structured rejected results, e.g. `backend_steering_unsupported`, `no_active_turn`, or `no_active_child_run`.
- Add `POST /attractor/pipelines/{pipeline_id}/steer`:
  - body: `message`, optional `target_run_id`, optional `target_node_id`
  - default target: active child from the parent checkpoint if present, otherwise the URL pipeline id
  - response: intervention result payload
  - publish `HumanInterventionRequested` on the parent run event stream.

## Tests
- Handler tests for manager-loop intervention request creation, artifact writing, context updates, and rejected no-child/no-backend cases.
- Backend tests for Codex app-server active-turn delivery, app-server steer errors, unified-agent session steering, provider-router forwarding, and unsupported backends.
- Container tests for `child_intervention_request` / `child_intervention_result` round-trip through local container execution.
- API tests for `/pipelines/{id}/steer`: empty message validation, default active-child targeting, explicit target override, and event publication.
- Run full suite with `uv run pytest -q`.

## Assumptions
- Automatic manager steering is conservative: it fires only from child failure context.
- If the child has already terminated and no active turn/session exists, the result is recorded as rejected instead of treated as delivery.
- `request_child_intervention` is an optional backend capability, not a required `CodergenBackend` protocol method.
- No frontend UI or CLI command is included in this change.
