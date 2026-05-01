# Remote Execution Workers Architecture

## Source Authority

The product contract for this implementation is `.spark/spec-implementation/current/spec/source.md`; the requirement ledger is `.spark/spec-implementation/current/spec/requirements.json`. This architecture binds implementation work to those files and to the machine-readable decisions in `.spark/spec-implementation/current/spec/contract-decisions.json`.

Remote execution is an execution placement for existing Spark/Attractor workflow runs. Spark remains the control plane of record. A remote worker is a prepared host-local execution service that admits runs, prepares run-scoped container state, executes individual node requests through the existing `spark-server worker run-node` process protocol, and reports results over HTTP/SSE.

## Canonical Repository Topology

New long-lived remote-execution code belongs under `src/attractor/execution/`.

Expected package layout:

```text
src/attractor/execution/
  __init__.py
  profiles.py          # execution profile/worker dataclasses or pydantic models
  config.py            # execution-profiles.toml loading and validation
  resolution.py        # selection precedence, disabled checks, launch snapshot
  paths.py             # path-segment-aware control-to-worker root mapping
  metadata.py          # run-record/context execution metadata helpers
  errors.py            # launch/admission/protocol error types and JSON error shape
  remote_client.py     # authenticated worker HTTP/SSE client
  remote_runner.py     # control-plane remote_worker orchestration
  worker_models.py     # worker API request/response/event models
  worker_app.py        # standalone FastAPI app factory/router for worker serve
  worker_state.py      # worker-side run/node lifecycle state and idempotency
  worker_runtime.py    # Docker/container/process lifecycle owned by the worker
  worker_bridge.py     # run-node process protocol bridge to worker events/results
```

Thin integration points:

- `src/attractor/api/server.py` resolves the selected execution profile at launch, snapshots metadata, dispatches to native, local_container, or remote_worker execution, records worker events in normal run history, and forwards human-gate/child-run callback results.
- `src/attractor/api/run_records.py` owns persisted run-record fields and serialization for execution placement metadata.
- `src/attractor/api/pipeline_runtime.py` remains the canonical in-process run/event/human-gate coordination layer; remote execution must call through it rather than creating a parallel run state store.
- `src/attractor/handlers/execution_container.py` remains compatibility glue for the existing local-container `spark-server worker run-node` process protocol. Shared protocol helpers may move into `src/attractor/execution/`, but this module must not become the remote-worker package.
- `src/spark/settings.py` remains the runtime-home source for `$SPARK_HOME`; execution profile config is read from `settings.config_dir / "execution-profiles.toml"`.
- `src/spark/workspace/storage.py` owns project metadata, including default `execution_profile_id`.
- `src/spark/workspace/api.py` owns workspace/project/launch API payloads for execution profile selection and UI-facing profile/settings data.
- `src/spark/chat/` owns chat-generated flow-run request artifacts and must emit `execution_profile_id` rather than direct image selection.
- `src/spark/cli.py` owns user-facing CLI run/launch flags and should expose `--execution-profile`.
- `src/spark/server_cli.py` owns `spark-server worker serve` and the existing `spark-server worker run-node` entrypoint. `worker serve` starts the standalone worker FastAPI app and is not mounted into the control-plane app.
- UI changes in `src/spark/app.py` and workspace routes should present execution placement in Settings, project settings, flow launch, and run details.

## Implementation Boundaries

### Execution Profile Configuration

`src/attractor/execution/config.py` loads `$SPARK_HOME/config/execution-profiles.toml` through the same settings/config-dir path used by the server process. Missing config is valid only when no configured project default or run override requires a profile; in that case Spark synthesizes a built-in native-compatible default profile.

The loader returns a normalized graph of workers and profiles:

- Worker ids and profile ids are stable keys from TOML tables.
- Profile modes are exactly `native`, `local_container`, and `remote_worker`.
- Profile `enabled` defaults to `true`; worker `enabled` is required.
- Disabled profiles and disabled workers stay visible to settings/debug surfaces but cannot be selected.
- Validation errors include the worker/profile id and failing field.
- Runtime reload can be simple and process-local, but each run launch must use one coherent validated graph and persist launch-time snapshots so later file edits do not rewrite run history.

### Selection And Dispatch

`src/attractor/execution/resolution.py` owns profile selection precedence:

1. explicit run `execution_profile_id`
2. project default `execution_profile_id`
3. Spark default `execution_profile_id`
4. implementation default compatible with existing native behavior

Public launch payloads, workspace API payloads, CLI commands, and chat-generated run requests use `execution_profile_id` as the durable selection field. The old `execution_container_image` request field is removed as a selection input; where existing run records/context expose `execution_container_image`, it is treated only as a resolved image snapshot for local_container and remote_worker runs.

`native` dispatch keeps the existing in-process `HandlerRunner` behavior. `local_container` dispatch uses the resolved profile image and the existing containerized runner/process protocol. `remote_worker` dispatch uses the remote HTTP/SSE transport and never asks DOT content to select workers.

### Root Mapping

`src/attractor/execution/paths.py` owns path translation. The control-plane project path must be under `control_project_root` using path-segment-aware matching. The worker project path is computed by preserving the exact relative suffix under `worker_project_root`.

Path mapping is a pure translation. It must not clone, fetch, archive, upload, sync, create, repair, mutate, or infer source state. If the control-plane path is outside the configured root, launch fails before contacting the worker. Worker-local existence checks happen at worker admission/preparation and are reported before node execution.

### Authentication And Worker Compatibility

`src/attractor/execution/remote_client.py` reads the bearer token from the selected worker's `auth_token_env` before any authenticated worker call. Missing or empty tokens fail launch before contacting authenticated endpoints. All worker requests, including SSE, send `Authorization: Bearer <token>`.

Health and worker-info endpoints support settings/debug visibility and compatibility checks. They must not become profile selection inputs. The control plane rejects incompatible v1 protocol versions under the launch policy.

### Worker API And Lifecycle

The worker service is a standalone FastAPI app under `src/attractor/execution/worker_app.py`, launched by `spark-server worker serve`.

The v1 worker API owns:

- `GET /v1/health`
- `GET /v1/worker-info`
- `POST /v1/runs`
- `GET /v1/runs/{run_id}`
- `POST /v1/runs/{run_id}/nodes`
- `GET /v1/runs/{run_id}/events`
- `POST /v1/runs/{run_id}/human-gates/{gate_id}/answer`
- `POST /v1/runs/{run_id}/child-runs/{request_id}/result`
- `POST /v1/runs/{run_id}/child-status/{request_id}/result`
- `POST /v1/runs/{run_id}/cancel`
- `DELETE /v1/runs/{run_id}`

All JSON errors use:

```json
{"error":{"code":"invalid_request","message":"...","retryable":false,"details":{}}}
```

`POST /v1/runs` is the admission boundary. It synchronously validates request shape, authorization, protocol compatibility, image policy, path policy, capability compatibility, immediate resource admission when cheap, and idempotency compatibility. It returns `202 Accepted` for asynchronous preparation and emits `run_ready` before node execution can be accepted. Preparation failure emits `run_failed` before any node executes.

The worker is authoritative for host-local Docker lifecycle, run-scoped container/runtime state, resource cleanup, and orphan cleanup policy. Spark is authoritative for routing, checkpoints, run records, UI event history, human gates, child-run provenance, and durable run metadata.

### Node Execution Bridge

`src/attractor/execution/worker_bridge.py` bridges accepted node execution to `spark-server worker run-node` inside the run-scoped execution container.

For each accepted node execution, the worker writes the node request to process stdin, reads stdout protocol messages, converts progress messages to SSE events, converts human-gate/child-run/child-status requests to SSE request events, waits for callback endpoint deliveries, writes answers/results back to process stdin, and emits either `node_result` or `node_failed`.

`node_result` preserves the current worker outcome contract: serialized `Outcome`, context updates, and runtime metadata. `node_failed` represents transport/process/contract failure that prevented a valid outcome from being produced. A valid `Outcome(status=FAIL)` remains a modeled result and is transported through `node_result`.

### SSE And Control-Plane Remote Execution

Worker SSE events use monotonic per-run sequence ids. The SSE `id` field is the sequence number and the SSE `event` field is the worker event type. Every event includes run id, sequence, event type, timestamp, worker id, execution profile id, and payload; node events also include node id and attempt when available.

The control plane admits a remote run, opens or resumes the event stream, waits for `run_ready`, submits one node request at a time for v1, applies `node_result` to canonical Attractor run state, and records failures for `node_failed`, worker `run_failed`, callback delivery exhaustion, and unrecoverable event gaps.

Duplicate SSE events are ignored by sequence id. Missing or gapped sequences fail the run unless `GET /v1/runs/{run_id}` proves no required run, node, gate, child-run, or terminal event was lost.

### Human Gates And Child Runs

Remote execution keeps human-gate and child-run authority in Spark. Worker events request decisions or child-run operations; the control plane records and surfaces them through existing UI/API paths, performs the authoritative action, and posts the result back to the worker callback endpoint.

Callback deliveries are idempotent by `gate_id` or `request_id`; compatible repeats succeed and conflicting repeats return `409 Conflict`. Delivery exhaustion after configured retries records active run failure.

### Failure Semantics

Failure handling is partitioned by the boundary where the failure can be observed and acted on:

- Launch validation fails in the control plane before worker contact for missing, disabled, or invalid selected profiles; missing or disabled referenced workers; unmappable control project paths; missing or empty auth tokens; and blocking health, worker-info, protocol, version, or capability validation.
- Run admission fails synchronously at `POST /v1/runs` before asynchronous preparation for rejected bearer tokens, unsupported protocol versions, malformed requests, forbidden images, incompatible capabilities, immediately detectable resource unavailability, and conflicting repeated admission requests.
- Worker preparation fails by emitting `run_failed` before node execution when mapped worker paths are missing, required mounts/devices/resources are unavailable, the run-scoped container cannot be created or reused, or image pull/availability checks fail after admission.
- Active run failure is recorded in normal Spark run state and event history for unrecoverable SSE gaps, worker `run_failed` after readiness, `node_failed`, callback delivery exhaustion, and remote transport failures that prevent a required result from being trusted.
- Cancellation sends the worker cancel request, stops new node submission, emits `run_canceling` and `run_canceled` when observed, interrupts active worker execution best-effort, and keeps cleanup available.
- Cleanup failures are visible in run details, worker snapshots, and logs without silently rewriting an already terminal workflow outcome unless a future explicit runtime policy says otherwise.
- Orphan cleanup is a worker-owned policy for admitted runs whose control plane disappears. It may be TTL-based, but cleanup decisions and errors must be visible through worker logs and run snapshots when Spark can still observe the worker.

All machine-readable failure payloads use stable codes, human-readable messages, retryable flags, and structured details. Launch/admission failures surface through the initiating API/CLI response; preparation and active-run failures surface through worker events and canonical run history; cleanup/orphan cleanup visibility is recorded where the worker or control plane can still report it.

### Runtime Metadata

Run records and seeded context persist launch-time snapshots:

- `execution_mode`
- `execution_profile_id`
- `execution_worker_id`
- `execution_worker_label`
- `execution_worker_base_url`
- `execution_container_image`
- `execution_mapped_project_path`
- `execution_worker_runtime_root`
- `execution_worker_version`
- `execution_worker_capabilities`
- `execution_profile_capabilities`

Native runs persist `execution_mode` and `execution_profile_id`; local_container and remote_worker runs persist the resolved profile image as `execution_container_image` snapshot metadata. Remote-worker launches must capture worker id, label, base URL, mapped project path, runtime root, worker version, worker capability snapshot, and profile capability snapshot before node execution. The selected worker's version and capabilities are required launch metadata obtained through the v1 compatibility/worker-info path; if the control plane cannot obtain compatible required identity, version, and capability data under the launch policy, launch fails before worker admission is treated as executable. Historical run records are not recomputed from later `execution-profiles.toml` edits.

### UI/API Fit

Settings exposes an Execution Workers section based on configured workers/profiles, validation errors, enabled states, health, worker-info, versions, status, and capabilities. Project settings exposes a default execution profile selector. Flow launch exposes effective defaults, optional run override selector, selected mode/profile label/id, and prelaunch validation state. Run details expose the launch-time execution metadata snapshot.

Remote execution is presented as execution placement, not as a different workflow type. Worker health and worker-info are observability inputs only, never selection authority.

## Public Interface

### Configuration File

The portable configuration boundary is:

```text
$SPARK_HOME/config/execution-profiles.toml
```

Public TOML shape:

```toml
[workers.<worker_id>]
label = "Human label"
base_url = "https://worker.example:8765"
auth_token_env = "SPARK_WORKER_TOKEN"
enabled = true

[profiles.<profile_id>]
mode = "native" | "local_container" | "remote_worker"
label = "Human label"
enabled = true
capabilities = []
```

`local_container` profiles require `image`. `remote_worker` profiles require `worker`, `image`, `control_project_root`, `worker_project_root`, and `worker_runtime_root`.

### Launch Selection

Durable selection field:

```json
{"execution_profile_id":"android-decompile"}
```

Direct launch selection by `execution_container_image` is not part of the public target-state contract. The image remains visible as a resolved run metadata snapshot.

### CLI

User-facing CLI launch/run commands expose `--execution-profile <profile_id>`. `spark-server worker serve` starts the standalone worker API. `spark-server worker run-node` remains the process-level node worker invoked inside execution containers.

### Worker HTTP/SSE API

The worker API is rooted under `/v1` and uses bearer-token authentication on every endpoint. REST requests initiate run, node, cancel, cleanup, and callback operations. SSE is the live worker-to-control-plane event/result stream.

## Dependency Order

1. `REQ-013` establishes package ownership and integration boundaries.
2. `REQ-001` implements config models/loading/validation.
3. `REQ-002` implements profile-id selection and removes direct image selection as public launch input.
4. `REQ-003` implements root mapping and launch metadata values needed by remote admission.
5. `REQ-004` implements auth, health, worker-info, and protocol compatibility checks.
6. `REQ-005`, `REQ-006`, and `REQ-007` implement worker API, process bridge, and SSE semantics.
7. `REQ-008`, `REQ-009`, `REQ-010`, and `REQ-014` implement control-plane remote orchestration, callbacks, metadata persistence, and failure/cancel/cleanup semantics.
8. `REQ-011` and `REQ-012` implement UI/API observability and DOT guardrails after the selection and metadata contracts exist.

## Validation Strategy

All validation must run through `uv run pytest`; repo-wide completion gates use `uv run pytest -q`. Narrow triage can use `uv run pytest -q -x --maxfail=1 <path-or-nodeid>`.

Tests should assert observable behavior through real interfaces:

- `tests/execution/` covers profile loading/validation, selection precedence, root mapping, auth headers, worker client, worker app endpoints, idempotency, SSE sequencing, process bridge, callback idempotency, and failure boundaries.
- `tests/api/` covers launch payloads, run records, run context, cancellation, human-gate/child-run forwarding, event history, settings/debug endpoints, and run details.
- `tests/handlers/test_execution_container.py` remains the compatibility suite for the `spark-server worker run-node` process contract.
- `tests/test_cli.py` covers `--execution-profile`, removal of direct image selection from public CLI launch surfaces, `spark-server worker serve`, and preserved `worker run-node`.
- `tests/contracts/frontend/` covers workspace/UI API payload shape for settings, project defaults, launch, and run details.
- `tests/dsl/` covers DOT non-authority guardrails if capability hints are introduced.
- `tests/repo_hygiene/` covers package-boundary expectations and prevents the worker API from being mounted into the control-plane app.

Acceptance tests must avoid source-text/prompt assertions and deprecated direct-image details. Compatibility tests may verify legacy run-record image snapshots where they are observable metadata, but not as a launch selection input.

## Repository Hygiene

- Keep remote execution code in `src/attractor/execution/`; do not create a separate top-level product or installable package.
- Keep server, workspace, app, chat, and CLI edits as integration surfaces over the execution runtime package.
- Do not mount worker routes into the normal Spark control-plane FastAPI app.
- Do not introduce wrapper-only delivery layers that duplicate the same selection or transport contract.
- Do not rely on hidden source preparation, test-only bootstrap behavior, environment-specific hacks, or compatibility shims as the primary execution path.
- Preserve existing native behavior when no configured execution profile is selected.
- Treat `execution_container_image` as snapshot metadata after resolution, not as durable public selection.
- Keep generated runtime state outside committed source and ensure tests create temporary `$SPARK_HOME` roots.
