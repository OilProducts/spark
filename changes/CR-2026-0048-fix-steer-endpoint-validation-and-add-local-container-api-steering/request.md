# Fix Steer Endpoint Validation and Add Local-Container API Steering

## Summary
Fix `POST /pipelines/{pipeline_id}/steer` so unknown parent pipeline IDs return the existing API behavior: HTTP `404` with `{"detail": "Unknown pipeline"}` and no event. Then add true API steering into an active local-container backend by extending the local container worker protocol with parent-initiated intervention messages.

## Key Changes
- In `steer_pipeline()`, validate the URL `pipeline_id` with the existing known-pipeline check before choosing a target or publishing events.
- Keep current behavior for known-but-not-active or not-steerable runs: return a `200` rejected intervention result such as `no_active_child_run`, `no_active_turn`, or `backend_steering_unsupported`.
- Extend the local-container transport with a thread-safe `request_child_intervention(...)` capability:
  - Parent/API calls into the active `ContainerizedHandlerRunner`.
  - The runner forwards the request to the active worker process.
  - The worker invokes its in-process `HandlerRunner.request_child_intervention(...)`, which reaches the active codergen backend when available.
  - The result is returned to the API caller and published in the existing `HumanInterventionRequested` event.
- Preserve existing worker-originated callback behavior for human gates, child runs, child status, and manager-loop child intervention.

## Container Protocol Design
- Add parent-initiated worker message type: `child_intervention_control_request`.
- Add worker response type: `child_intervention_control_result`.
- Include a generated `request_id` so the parent can match responses to concurrent API steering requests.
- Refactor worker stdin handling so a single reader owns stdin and dispatches:
  - existing callback responses to the blocking callback waiters
  - parent-initiated intervention requests to the active worker runner/backend
- If no worker process is active, or no active backend/turn/session exists inside the worker, return a structured rejected result rather than failing the run.

## Tests
- API tests:
  - unknown `pipeline_id` on `/steer` returns `404` and `{"detail": "Unknown pipeline"}`
  - no `HumanInterventionRequested` event is published for unknown parent IDs
  - known but inactive/non-steerable runs still return `200` rejected intervention results
- Container tests:
  - parent-initiated `request_child_intervention(...)` forwards into the active worker and returns the worker result
  - request IDs route multiple outstanding intervention responses correctly
  - inactive container worker returns a rejected result without hanging
  - existing worker-originated child intervention callback path still works
- Backend regression tests:
  - Codex app-server active-turn steering still uses `turn/steer`
  - unified-agent active-session steering still calls `Session.steer`
- Run `uv run pytest -q`.

## Assumptions
- API steering into local containers is only guaranteed while a node worker process is actively running.
- If the active container worker is between nodes or already finished, the response should be rejected with a non-fatal reason such as `no_active_turn` or `no_active_container_worker`.
- This change does not add remote-worker support and does not change the public `/steer` request or response schema.
