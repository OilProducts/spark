# Remote Execution Workers

This document is the canonical target-state contract for executing Spark workflow runs on prepared remote worker hosts.

It defines the remote-worker execution model, configuration contract, worker API, transport semantics, runtime metadata, and UI fit. It does not specify an implementation that exists today.

## 1. Purpose

Spark can run workflow nodes in multiple execution modes:
- `native`
- `local_container`
- `remote_worker`

Remote execution extends Spark execution profiles. It lets the Spark control plane coordinate a run while delegating node execution to a pre-provisioned worker host that owns host-local Docker and resource concerns.

Remote workers are for prepared infrastructure. They are not a source checkout, sync, or provisioning feature.

## 2. Scope and Non-Goals

This specification defines:
- profile and worker configuration shape
- profile selection semantics
- project-root mapping semantics
- worker authentication expectations
- remote worker HTTP and SSE surfaces
- event ordering and reconnect requirements
- control-plane callback responsibilities
- run metadata that Spark must persist and surface
- UI placement for worker/profile observability
- future acceptance scenarios

This specification does not implement:
- worker server routes
- Spark config loading
- UI management
- run launching changes
- container runtime behavior
- source synchronization
- remote host provisioning

## 3. Authority Boundaries

Spark remains the control plane of record.

Spark is authoritative for:
- flow routing
- checkpoints
- run records
- UI event history
- human gates
- child-run coordination
- profile selection at launch
- durable run metadata
- parent/child run provenance

The remote worker is authoritative for:
- host-local Docker lifecycle
- worker-local path validation
- worker-local mounts, devices, and resources
- execution-container cleanup
- executing `spark-server worker run-node`
- worker-side runtime state for the run-scoped execution container

The worker must not become the workflow control plane. It executes requested nodes and reports events/results. It does not decide graph routing, persist canonical checkpoints, answer human gates, or create authoritative child-run records.

## 4. No Hidden Project Preparation

Remote execution must not perform hidden source preparation before the first flow node.

The remote worker and control plane must not implicitly:
- clone a git repository
- pull or fetch repository updates
- upload an archive
- sync files
- mutate project source
- repair missing directories
- run setup commands outside the requested node execution contract

Remote workers and remote profiles must be pre-provisioned. Source preparation belongs in one of these places:
- an explicit flow node
- a Dockerfile or image build
- operator setup outside Spark
- another documented provisioning system invoked intentionally by the operator

Root mappings are path translations only. They are not sync or provisioning instructions.

## 5. Execution Profiles

Spark execution profiles describe where and how nodes execute.

### 5.1 Modes

`native` runs in the control-plane process environment.

`local_container` runs in a local execution container managed by the control-plane host.

`remote_worker` runs nodes through an HTTP/SSE worker transport on a prepared remote worker host. The remote worker creates or reuses a run-scoped execution container and executes each node through `spark-server worker run-node`.

### 5.2 Selection Model

Projects and run launches select an execution profile by `execution_profile_id`.

They must not select workers or images directly. Worker id, image, mapped root, runtime root, and capabilities are resolved from the selected profile and captured into run metadata at launch.

Profile-based execution replaces direct `execution_container_image` selection. Project defaults, run launch requests, CLI commands, Workspace API payloads, and chat-generated run requests should use `execution_profile_id` as the durable selection field. The old direct-image selection path should be removed rather than maintained as a parallel public contract.

Selection precedence is:
1. explicit run override
2. project default execution profile
3. Spark default execution profile
4. implementation default, which must remain compatible with existing native behavior

Disabled profiles cannot be selected at any precedence level.

## 6. Configuration Contract

The initial source of truth is:

```text
$SPARK_HOME/config/execution-profiles.toml
```

Future UI management may edit or generate this file, but the file contract is the target-state portable configuration boundary.

Spark should resolve `$SPARK_HOME` through the same runtime settings path used by the existing server process. Missing `execution-profiles.toml` is valid only when Spark can synthesize a built-in native default profile. Any configured project default or run override that references a missing profile must fail before launch.

The config loader should be deterministic and process-local:
- read the file from the active Spark runtime home
- normalize and validate all profiles before use
- expose validation errors to settings and launch surfaces
- avoid silent fallback from an invalid selected profile to another profile
- snapshot resolved profile values into run records at launch

Runtime reload behavior is an implementation choice, but run launch must use one coherent validated profile graph. Existing in-flight runs must continue using their launch-time metadata snapshot even if `execution-profiles.toml` changes later.

### 6.1 Conceptual Shape

```toml
[workers.android-lab]
label = "Android Lab"
base_url = "https://android-lab.internal:8765"
auth_token_env = "SPARK_WORKER_ANDROID_LAB_TOKEN"
enabled = true

[profiles.android-decompile]
mode = "remote_worker"
label = "Android decompile lab"
worker = "android-lab"
image = "spark-android-tools:2026-05"
control_project_root = "/home/chris/projects"
worker_project_root = "/work/projects"
worker_runtime_root = "/work/spark-runtime"
capabilities = ["docker", "android-emulator", "apktool", "jadx"]

[profiles.local-spark]
mode = "local_container"
label = "Local Spark container"
image = "spark:package"

[profiles.native]
mode = "native"
label = "Native"
```

### 6.2 Worker Fields

Worker ids are stable identifiers. They are used in profile references, persisted run records, health/debug surfaces, and operator-facing logs.

Worker fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `label` | yes | Human-readable worker label for UI and logs. |
| `base_url` | yes | HTTPS base URL for the worker API. |
| `auth_token_env` | yes | Environment variable read by the Spark control plane to obtain the bearer token. |
| `enabled` | yes | Disabled workers cannot be selected through any profile. |

Worker `base_url` must identify one worker service. It must not point at a generic load-balanced pool unless the pool preserves run affinity for all `/v1/runs/{run_id}` requests and SSE streams.

### 6.3 Profile Fields

Profile ids are stable identifiers. They are persisted as `execution_profile_id` on run records and should not be reused for semantically different execution environments.

Common profile fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `mode` | yes | One of `native`, `local_container`, or `remote_worker`. |
| `label` | yes | Human-readable profile label for selection and logs. |
| `enabled` | optional | Defaults to `true`; disabled profiles cannot be selected. |
| `capabilities` | optional | Operator-authored capability hints captured into run metadata. |

`local_container` fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `image` | yes | Container image used by local container execution. |

`remote_worker` fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `worker` | yes | Worker id under `[workers.*]`. |
| `image` | yes | Worker-local execution container image. |
| `control_project_root` | yes | Control-plane root prefix used for path mapping. |
| `worker_project_root` | yes | Worker-side root prefix used for mapped project paths. |
| `worker_runtime_root` | yes | Worker-side location for Spark runtime state and run-scoped container mounts. |
| `capabilities` | optional | Profile-level capability hints expected from this environment. |

`native` profiles do not require an image, worker, or root mapping.

### 6.4 Validation Rules

Configuration loading must validate before any run launch uses the profile graph.

Rules:
- profile ids and worker ids are stable identifiers
- worker ids must be unique
- profile ids must be unique
- profile `mode` must be one of `native`, `local_container`, or `remote_worker`
- disabled workers cannot be selected
- disabled profiles cannot be selected
- `remote_worker` profiles must reference an existing enabled worker
- `remote_worker` profiles require `image`, `control_project_root`, `worker_project_root`, and `worker_runtime_root`
- `local_container` profiles require `image`
- `native` profiles must not require worker/image fields
- root mappings are pure path translations, not sync/provisioning instructions
- if the control-plane project path is outside `control_project_root`, run launch fails before execution
- if the mapped worker project path does not exist, worker preparation fails before node execution
- `auth_token_env` must resolve to a non-empty token before the control plane calls authenticated worker endpoints

Validation errors must identify the profile id or worker id that failed validation and the field that made launch impossible.

## 7. Root Mapping

Remote workers map the control-plane project path into the worker filesystem by replacing the configured control root prefix with the configured worker root prefix.

Example:

```text
control_project_root = /home/chris/projects
worker_project_root  = /work/projects
control project path = /home/chris/projects/example-app
mapped worker path   = /work/projects/example-app
```

Rules:
- matching is path-segment aware, not raw string prefix matching
- the control project path must be inside `control_project_root`
- the relative suffix below `control_project_root` is preserved exactly
- path translation does not create directories
- path translation does not copy files
- path translation does not infer git remotes or branches
- the worker validates the mapped path exists before node execution

If the mapped path does not exist on the worker, the worker must report failure before any node executes. The failure may be returned synchronously from run admission or emitted asynchronously as a run-preparation failure.

### 7.1 Execution Container Paths

The worker-side execution container should see the mapped project path at the same absolute path used by the worker.

Example:

```text
worker project path      = /work/projects/example-app
container project path   = /work/projects/example-app
worker runtime root      = /work/spark-runtime
container runtime root   = /work/spark-runtime
```

The worker may implement this with bind mounts, named volumes, or another Docker mechanism, but node execution must receive stable paths that match the run metadata. Spark node commands should not need to know whether they are running in a local container or a remote worker container.

The worker must not silently remap project paths to unrelated container-only paths unless that remapping is part of the explicit worker protocol and persisted in run metadata.

## 8. Authentication

Every worker endpoint requires bearer-token authentication:

```http
Authorization: Bearer <token>
```

The Spark control plane reads the token from the environment variable named by the worker's `auth_token_env`.

V1 remote workers are trusted, pre-provisioned hosts on private infrastructure. Bearer-token auth is sufficient for v1. Mutual TLS may be added later without changing the execution profile model or the worker API resource shape.

Workers must reject missing, malformed, or invalid tokens with `401 Unauthorized` or `403 Forbidden` as appropriate. They must not accept unauthenticated SSE streams.

## 9. Remote Worker API

The v1 worker API is rooted under `/v1`. The worker exposes these endpoints:

```http
GET /v1/health
GET /v1/worker-info
POST /v1/runs
GET /v1/runs/{run_id}
POST /v1/runs/{run_id}/nodes
GET /v1/runs/{run_id}/events
POST /v1/runs/{run_id}/human-gates/{gate_id}/answer
POST /v1/runs/{run_id}/child-runs/{request_id}/result
POST /v1/runs/{run_id}/child-status/{request_id}/result
POST /v1/runs/{run_id}/cancel
DELETE /v1/runs/{run_id}
```

All JSON error responses use this shape:

```json
{
  "error": {
    "code": "resource_unavailable",
    "message": "worker has no available emulator slots",
    "retryable": true,
    "details": {}
  }
}
```

Common error codes include:
- `invalid_request`
- `unauthorized`
- `forbidden`
- `not_found`
- `conflict`
- `resource_unavailable`
- `worker_unavailable`
- `preparation_failed`
- `execution_failed`
- `protocol_error`

### 9.1 `GET /v1/health`

Returns worker identity and liveness information.

Response fields:
- worker id
- worker version
- protocol version
- status
- capabilities
- optional diagnostic message

Status values should distinguish at least healthy, degraded, and unavailable states.

The control plane must reject workers that do not advertise a compatible protocol version.

### 9.2 `GET /v1/worker-info`

Returns worker-local metadata and capabilities for UI/debug validation.

The response may include:
- supported images
- worker-local capabilities
- resource labels
- version details
- worker-local validation information

The control plane does not select execution profiles from this endpoint. Spark still selects by configured `execution_profile_id`; this endpoint exists for operator visibility and compatibility checks.

### 9.3 `POST /v1/runs`

Admits a run on the worker and starts worker-side preparation for a run-scoped execution container.

The request identifies:
- Spark run id
- execution profile id
- requested image
- mapped worker project path
- worker runtime root
- capabilities snapshot
- launch-time metadata snapshot

Example request:

```json
{
  "run_id": "091ba61b8b674ce6a6228f4f2372ace7",
  "execution_profile_id": "android-decompile",
  "image": "spark-android-tools:2026-05",
  "mapped_project_path": "/work/projects/example-app",
  "worker_runtime_root": "/work/spark-runtime",
  "capabilities": ["docker", "android-emulator", "apktool", "jadx"],
  "metadata": {
    "project_id": "example-app",
    "control_project_path": "/home/chris/projects/example-app"
  }
}
```

`POST /v1/runs` is the run-admission boundary. It must synchronously reject invalid, unauthorized, impossible, or incompatible requests when the worker can determine that cheaply. It does not have to block on long-running preparation work such as image pulls or container creation.

Synchronous validation should include:
- request shape and required fields
- bearer-token authorization
- protocol compatibility
- image policy
- root mapping syntax and mapped path policy
- required capability compatibility
- resource admission when the worker can decide immediately
- idempotency compatibility for an existing worker-side run

Long-running preparation may happen asynchronously after admission. That includes:
- image pull or image availability checks that take wall-clock time
- mount setup
- device allocation
- run-scoped container creation
- worker-runtime directory setup
- mapped project path existence checks when they depend on worker-local runtime state

Creation is idempotent for the same run id and compatible inputs. If a repeated request supplies incompatible inputs for an existing worker-side run, the worker must reject it.

Successful admission returns `202 Accepted`:

```json
{
  "run_id": "091ba61b8b674ce6a6228f4f2372ace7",
  "worker_id": "android-lab",
  "status": "preparing",
  "event_url": "/v1/runs/091ba61b8b674ce6a6228f4f2372ace7/events",
  "last_sequence": 0
}
```

The worker reports preparation progress and failures through the event stream. The worker must emit `run_ready` before accepting node execution. If preparation fails, it must emit `run_failed` with a structured reason before any node executes.

### 9.4 `GET /v1/runs/{run_id}`

Returns the worker-side run snapshot.

The snapshot should include:
- worker-side run status
- active node id, when any
- container id or opaque container handle
- image
- mapped worker project path
- last emitted event sequence
- worker version/capabilities snapshot
- last error, when any

The control plane may use this endpoint to inspect worker-side state after reconnect, but v1 does not require full event-log reconciliation. If the control plane detects a missing SSE sequence and cannot prove from the snapshot that no required run, node, gate, child-run, or terminal event was lost, it must fail the run rather than invent state.

### 9.5 `POST /v1/runs/{run_id}/nodes`

Executes one flow node inside the run-scoped execution container through:

```text
spark-server worker run-node
```

The request includes:
- `node_execution_id`, stable for this control-plane request
- node id
- node attempt
- the node execution payload needed by the existing worker outcome contract
- context
- node metadata
- runtime references
- bounded execution options

The worker streams progress through `GET /v1/runs/{run_id}/events`. The HTTP response acknowledges that node execution was accepted or rejected; final node outcome is delivered as `node_result` or `node_failed` on the event stream.

Repeated requests with the same `node_execution_id` and compatible payload are idempotent and return the existing accepted execution state. Repeated requests with the same `node_execution_id` and conflicting payload must be rejected with `409 Conflict`.

The worker must reject node execution before `run_ready`.

Only one active node execution per worker-side run is required for v1 unless a later profile explicitly supports parallel node execution.

### 9.6 `GET /v1/runs/{run_id}/events`

Provides a Server-Sent Events stream for worker-to-control-plane events.

The stream carries:
- worker events
- node events
- stdout/stderr summaries
- human-gate requests
- child-run requests
- child-status requests
- final node results
- cancellation and cleanup events

SSE events must carry monotonic per-run sequence numbers. The SSE `id` field is the sequence number and the SSE `event` field is the worker event type.

Example event:

```text
id: 42
event: node_event
data: {"run_id":"091ba61b8b674ce6a6228f4f2372ace7","sequence":42,"event_type":"node_event","timestamp":"2026-05-01T13:22:00Z","worker_id":"android-lab","execution_profile_id":"android-decompile","node_id":"test","node_attempt":1,"payload":{"message":"running tests"}}
```

Reconnect uses either:
- `Last-Event-ID`
- `?after=<sequence>`

Duplicate events are ignored by sequence id. Missing or gapped events cause the control plane to mark the run failed unless `GET /v1/runs/{run_id}` proves that no required run, node, gate, child-run, or terminal event was lost.

The SSE stream is the source of live run-view updates for remote execution and the delivery path for final node outcomes. Sequence handling is therefore both a UI continuity concern and a correctness concern.

### 9.7 `POST /v1/runs/{run_id}/human-gates/{gate_id}/answer`

Delivers a control-plane answer to a human-gate request emitted by the worker.

The control plane remains authoritative for the human-gate decision and audit record. The worker only receives the answer needed to let `spark-server worker run-node` continue.

Delivery is idempotent by `gate_id`. Repeated compatible answers are accepted; conflicting answers are rejected with `409 Conflict`.

### 9.8 `POST /v1/runs/{run_id}/child-runs/{request_id}/result`

Delivers the result of a child-run request that required control-plane authority.

Child-run creation, identity, provenance, and final coordination remain control-plane responsibilities.

Delivery is idempotent by `request_id`. Repeated compatible results are accepted; conflicting results are rejected with `409 Conflict`.

### 9.9 `POST /v1/runs/{run_id}/child-status/{request_id}/result`

Delivers the result of a child-status request that required control-plane authority.

The worker may request status because the executing node needs it, but the authoritative child-run state remains in Spark/Attractor run records.

Delivery is idempotent by `request_id`. Repeated compatible results are accepted; conflicting results are rejected with `409 Conflict`.

### 9.10 `POST /v1/runs/{run_id}/cancel`

Interrupts active node execution and marks the worker-side run as canceling.

Cancellation should:
- signal the active `spark-server worker run-node` process
- stop accepting new node execution for the run
- emit `run_canceling`
- emit `run_canceled` when interruption and cleanup reach the canceled state

Cancellation is best-effort for already-running host processes, but the worker must make container cleanup available through `DELETE /v1/runs/{run_id}`.

### 9.11 `DELETE /v1/runs/{run_id}`

Removes the run-scoped execution container and worker-side runtime state.

The endpoint must be idempotent. Deleting an already-removed worker-side run should return success or a non-error empty state.

## 10. Transport Semantics

The control-plane transport is HTTP plus SSE:
- REST requests initiate run, node, cancel, and cleanup operations
- SSE is the live event stream from worker to control plane
- SSE sequence numbers are monotonic per run
- reconnect uses `Last-Event-ID` or `?after=<sequence>`
- duplicate events are ignored by sequence id
- missing/gapped events cause the control plane to mark the run failed unless `GET /v1/runs/{run_id}` proves that no required event was lost

Required worker event types:

```text
run_started
run_preparing
image_pull_started
image_pull_progress
container_creating
run_ready
node_started
node_event
human_gate_request
child_run_request
child_status_request
node_result
node_failed
run_canceling
run_canceled
run_failed
run_closed
worker_log
```

Every event must include:
- run id
- sequence number
- event type
- timestamp
- worker id
- execution profile id
- payload

Events related to a node should also include node id and node attempt when available.

`run_ready` is the only event that makes the run eligible to accept node execution. Preparation events before `run_ready` are observable progress only.

`run_failed` payloads must include a stable failure code, a human-readable message, whether retry may succeed, and optional structured details.

## 11. Control-Plane Execution Sequence

For a remote-worker run, the Spark control plane should execute this sequence:

1. Resolve `execution_profile_id` using the configured precedence.
2. Validate the selected profile and referenced worker.
3. Translate the control-plane project path to the mapped worker project path.
4. Persist launch-time execution metadata in the run record.
5. Create a remote execution transport for the selected worker.
6. Call `POST /v1/runs`.
7. Open or resume `GET /v1/runs/{run_id}/events`.
8. Wait for `run_ready` before submitting any node.
9. Submit each control-plane node execution with `POST /v1/runs/{run_id}/nodes`.
10. Apply `node_result` or `node_failed` events to canonical Attractor run state.
11. Surface worker events and logs into the normal run event history.
12. Forward human-gate answers, child-run results, and child-status results back to the worker when requested.
13. On cancellation, call `POST /v1/runs/{run_id}/cancel`.
14. On cleanup, call `DELETE /v1/runs/{run_id}` according to the run cleanup policy.

The control plane remains responsible for deciding graph routing after each node result. The worker must not infer the next node to run.

The control plane must not submit a node before `run_ready`. If `run_failed` arrives before `run_ready`, launch/preparation fails and no node result is applied.

## 12. Worker Process Bridge

The remote worker server bridges the HTTP/SSE protocol to the existing `spark-server worker run-node` process protocol.

For each accepted node execution, the worker:
1. starts or reuses the run-scoped execution container
2. executes `spark-server worker run-node` inside that container
3. writes the node execution request to the process stdin
4. reads process stdout messages
5. converts process event messages into SSE events
6. converts process human-gate and child-run requests into SSE request events
7. waits for the control plane to deliver answers/results through the worker callback endpoints
8. writes those answers/results back to the process stdin
9. converts the final process result into `node_result`
10. converts process crashes, invalid process messages, or missing final results into `node_failed`

This bridge is the compatibility boundary between the current local-container worker process protocol and the remote-worker HTTP/SSE protocol. The worker server owns Docker lifecycle and process management; the control plane owns workflow state and routing.

## 13. Node Outcome Contract

The `node_result` payload must preserve the current worker outcome shape:
- serialized `Outcome`
- context updates
- runtime metadata needed by the control plane

The control plane applies the result to canonical run state. The worker does not update control-plane checkpoints directly.

`node_failed` represents execution failure, transport-adjacent failure, or worker-side contract failure that prevented a valid `Outcome` from being produced. A valid `Outcome(status=FAIL)` remains a modeled node result and should be transported through `node_result` unless the existing worker outcome contract says otherwise.

## 14. Human Gates and Child Runs

Remote execution does not move human-gate or child-run authority to the worker.

When node execution needs a human gate:
1. the worker emits `human_gate_request`
2. the control plane records and surfaces the gate
3. the operator answers through Spark UI or API
4. the control plane calls `/v1/runs/{run_id}/human-gates/{gate_id}/answer`
5. the worker resumes the node process with the answer

When node execution needs a child run:
1. the worker emits `child_run_request`
2. the control plane launches and records the child run
3. the control plane reports the result through `/v1/runs/{run_id}/child-runs/{request_id}/result`

When node execution needs child status:
1. the worker emits `child_status_request`
2. the control plane reads authoritative child-run state
3. the control plane reports the result through `/v1/runs/{run_id}/child-status/{request_id}/result`

## 15. Runtime Metadata

Run records must persist:
- `execution_mode`
- `execution_profile_id`
- `execution_worker_id`
- execution container image snapshot
- mapped worker project path
- worker version snapshot
- worker capability snapshot
- profile capability snapshot

Run context must seed the same execution metadata for observability.

For `local_container` and `remote_worker`, the execution container image snapshot captures the resolved profile image used for node execution. This is run metadata, not a selection input.

Run details should preserve the selected values as launch-time snapshots. Later edits to `execution-profiles.toml` must not rewrite historical run metadata.

## 16. UI Fit

Settings gets an `Execution Workers` section showing:
- configured workers
- configured profiles
- worker health
- worker status
- worker capabilities
- worker version
- whether a worker/profile is enabled
- validation errors that prevent selection

Project settings gets a default execution profile selector.

Flow launch UI shows:
- effective project default execution profile
- optional run override selector
- selected mode
- selected profile label/id
- validation state before launch

Run details show:
- execution mode
- execution profile id
- worker id
- worker URL label
- image
- mapped worker path
- worker version snapshot
- capabilities snapshot

The UI should make remote execution visible as an execution placement choice, not as a different workflow type.

## 17. Flow DOT Fit

Flow DOT does not select remote workers in v1.

Future DOT capability hints may be allowed, but they are not binding worker selection. Spark resolves the actual execution profile from project and run launch state.

DOT-authored hints must not override a disabled profile, select a worker directly, or make an unavailable root mapping valid.

## 18. Implementation Placement

Remote execution should be implemented as part of the Attractor execution runtime, not as a separate top-level product or a Spark workspace feature.

The preferred package boundary is:

```text
src/attractor/execution/
```

This package should own:
- execution profile models and resolution
- `execution-profiles.toml` loading and validation
- execution metadata/context seeding
- local-container execution transport
- remote-worker HTTP/SSE client protocol
- remote-worker request/response/event models
- remote-worker standalone FastAPI app and API router
- worker-side run state and lifecycle management

Existing integration points should remain thin:
- `src/attractor/api/server.py` resolves the selected execution profile and dispatches to native, local-container, or remote-worker execution.
- `src/attractor/handlers/execution_container.py` may remain as compatibility glue during migration, but should not grow into the long-term remote execution package.
- `src/spark/workspace/` owns project defaults, launch payloads, and workspace API exposure for execution profile selection.
- `src/spark/app.py` and `src/spark/workspace/api.py` own UI-facing routes and settings surfaces.
- `src/spark/server_cli.py` owns operator commands such as `spark-server worker serve` and the existing `spark-server worker run-node` entrypoint.

`spark-server worker serve` should start a standalone worker FastAPI app. It should not mount the worker API into the normal Spark control-plane app. A host may run both the Spark control plane and a worker service, but they remain separate services with separate authority boundaries.

The remote worker implementation should ship in the same Python distribution as Spark. It should not become a separate installable package unless there is a later deployment requirement that justifies splitting release, versioning, and dependency management.

## 19. Failure Semantics

Launch must fail before contacting a worker when:
- the selected profile does not exist
- the selected profile is disabled
- the selected remote worker is disabled
- the selected remote worker is missing
- the control-plane project path cannot be mapped under `control_project_root`
- the bearer token environment variable is missing or empty
- worker health or worker-info validation blocks launch under the chosen policy

Run admission must fail before asynchronous preparation when:
- the worker rejects the bearer token
- the worker does not support the required v1 protocol
- the request is malformed
- the requested image is forbidden by worker policy
- the requested capabilities are incompatible with the worker
- the worker can cheaply determine that required resources are unavailable
- repeated `POST /v1/runs` inputs conflict with an existing worker-side run

Worker preparation must report `run_failed` before node execution when:
- the mapped worker project path does not exist
- required worker-local mounts/devices/resources are unavailable
- the run-scoped container cannot be created or reused
- image pull or image availability checks fail after admission

Active run failure must be recorded when:
- SSE reconnect cannot recover a missing event gap
- the worker reports `run_failed`
- node execution reports `node_failed`
- callback delivery cannot complete within the configured retry policy

Cleanup failure should be visible in run details and logs, but a cleanup failure after a terminal run result must not silently rewrite the workflow outcome without an explicit runtime policy.

Workers should support an orphan cleanup policy for admitted runs whose control plane disappears before cancellation or cleanup. The policy may be TTL-based, but cleanup must be visible through worker logs and run snapshots when Spark can still observe the run.

## 20. Future Acceptance Scenarios

Later implementation should prove these scenarios through observable behavior:
- profile config loads and validates valid and invalid TOML
- disabled workers and profiles cannot be selected
- `remote_worker` profiles must reference enabled workers
- project default profile and run override resolve deterministically
- existing native behavior continues to work unchanged
- profile-based local-container execution replaces direct `execution_container_image` selection
- mapped project paths are computed without cloning or syncing
- launch fails clearly when root mapping is impossible
- worker preparation fails before node execution when the mapped worker path is missing
- worker health and worker-info endpoints are consumed by Settings
- `POST /v1/runs` returns `202 Accepted` for admitted asynchronous preparation
- preparation progress and preparation failure are visible through SSE events
- node execution is rejected before `run_ready`
- control-plane remote execution waits for `run_ready` before submitting nodes
- worker process bridge converts `spark-server worker run-node` process messages into remote SSE events and final node outcomes
- run launch persists execution mode, profile id, worker id, image, mapped path, worker version, and capabilities snapshot
- node execution uses HTTP/SSE transport
- worker SSE event ordering is preserved by sequence id
- duplicate SSE events are ignored
- missing SSE event gaps fail the run unless `GET /v1/runs/{run_id}` proves that no required event was lost
- `node_result` preserves serialized `Outcome`, context updates, and runtime metadata
- human gates remain control-plane recorded and answered
- child runs and child-status lookups remain control-plane coordinated
- cancellation interrupts active worker execution and exposes run-container cleanup
- `DELETE /v1/runs/{run_id}` removes worker-side run state idempotently

## 21. Assumptions

V1 remote workers are trusted, pre-provisioned hosts on private infrastructure.

Operators are responsible for making source trees, Docker images, devices, and host resources available before selecting remote execution.

Bearer-token auth is sufficient for v1. Stronger transport authentication may be added later without changing the high-level execution profile model.

Remote worker implementation, UI, API routes, config loader, and tests are separate follow-up work.
