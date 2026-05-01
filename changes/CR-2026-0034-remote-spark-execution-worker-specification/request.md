# Remote Spark Execution Worker Specification

## Summary

Create a canonical spec document for remote Spark execution workers, without implementing runtime behavior yet.

Add `specs/remote-execution-workers.md` and update `specs/README.md` to list it as the target-state contract for executing Spark workflow runs on prepared remote worker hosts. The document should be detailed enough to drive a later implementation without re-deciding endpoints, transport, profile selection, root mapping, auth, or UI fit.

## Document Scope

- Define remote execution as an extension of Spark execution profiles:
  - `native`
  - `local_container`
  - `remote_worker`
- State that Spark control plane remains authoritative for:
  - flow routing
  - checkpoints
  - run records
  - UI event history
  - human gates
  - child-run coordination
- State that the remote worker is authoritative for:
  - host-local Docker lifecycle
  - worker-local path validation
  - worker-local mounts/devices/resources
  - execution-container cleanup
  - executing `spark-server worker run-node`
- Explicitly prohibit hidden project preparation:
  - no implicit git clone
  - no implicit git pull
  - no implicit archive sync
  - no implicit mutation before the first flow node
- Require remote workers/profiles to be pre-provisioned. Source preparation belongs in a flow node, Dockerfile, or operator setup.

## Configuration Contract

Document a `$SPARK_HOME/config/execution-profiles.toml` file as the initial source of truth, with future UI management expected.

Required conceptual shape:

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

Specify validation rules:

- profile ids and worker ids are stable identifiers
- disabled workers/profiles cannot be selected
- `remote_worker` profiles must reference an enabled worker
- `remote_worker` profiles require `image`, `control_project_root`, `worker_project_root`, and `worker_runtime_root`
- root mappings are pure path translations, not sync/provisioning instructions
- if the control-plane project path is outside `control_project_root`, run launch fails before execution
- if the mapped worker path does not exist, worker start fails before node execution

Project/run selection should use `execution_profile_id`, not worker/image fields directly. The existing `execution_container_image` should be treated as the local-container v1 compatibility field until migrated.

## Remote Worker API

Specify bearer-token auth for every endpoint:

```http
Authorization: Bearer <token>
```

Define these worker endpoints:

```http
GET /health
GET /profiles
POST /runs
GET /runs/{run_id}
POST /runs/{run_id}/nodes
GET /runs/{run_id}/events
POST /runs/{run_id}/human-gates/{gate_id}/answer
POST /runs/{run_id}/child-runs/{request_id}/result
POST /runs/{run_id}/child-status/{request_id}/result
POST /runs/{run_id}/cancel
DELETE /runs/{run_id}
```

Endpoint behavior:

- `GET /health` returns worker id, version, status, and capabilities.
- `GET /profiles` returns worker-local profile metadata/capabilities for UI/debug validation.
- `POST /runs` creates or reuses a run-scoped execution container on the worker.
- `POST /runs/{run_id}/nodes` executes one node through `spark-server worker run-node` inside the run container.
- `GET /runs/{run_id}/events` is an SSE stream for worker events, node events, stdout/stderr summaries, human-gate requests, child-run requests, child-status requests, and final node results.
- callback/result endpoints let the Spark control plane answer worker requests that require control-plane authority.
- `POST /runs/{run_id}/cancel` interrupts active node execution and marks the worker-side run as canceling.
- `DELETE /runs/{run_id}` removes the run-scoped container and worker-side runtime state.

## Transport Semantics

Document the control-plane transport as HTTP plus SSE:

- REST requests initiate run/node/cancel/cleanup operations.
- SSE is the live event stream from worker to control plane.
- SSE events must carry monotonic per-run sequence numbers.
- reconnect uses `Last-Event-ID` or `?after=<sequence>`.
- duplicate events are ignored by sequence id.
- missing/gapped events cause the control plane to mark the run failed unless a later worker snapshot reconciles it explicitly.

Required worker event types:

```text
run_started
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

The `node_result` payload must preserve the current worker outcome shape: serialized `Outcome`, context updates, and any runtime metadata needed by the control plane.

## UI and Runtime Fit

Document UI placement:

- Settings gets an “Execution Workers” section showing configured workers/profiles, health, status, capabilities, and version.
- Project settings gets a default execution profile selector.
- Flow launch UI shows the effective project default and allows a run override.
- Run details show execution mode, profile id, worker id, worker URL label, image, mapped worker path, and capabilities snapshot.

Document runtime metadata:

- run records persist `execution_mode`, `execution_profile_id`, `execution_worker_id`, `execution_container_image`, mapped worker project path, and worker version/capability snapshot.
- run context seeds the same execution metadata for observability.
- flow DOT does not select remote workers in v1.
- future DOT capability hints may be allowed, but they are not binding worker selection.

## Test/Acceptance Guidance For Later Implementation

The spec should include future acceptance scenarios:

- profile config loads and validates valid/invalid TOML.
- project default profile and run override resolve deterministically.
- mapped project paths are computed without cloning/syncing.
- launch fails clearly when mapping is impossible.
- worker health/profile endpoints are consumed by Settings.
- node execution uses HTTP/SSE transport and preserves event ordering.
- human gates and child runs remain control-plane coordinated.
- cancellation interrupts active worker execution and cleans up run containers.
- native and local-container modes continue to work unchanged.

## Assumptions

- This task creates documentation only.
- Remote worker implementation, UI, API routes, config loader, and tests are separate follow-up work.
- V1 remote workers are trusted, pre-provisioned hosts on private infrastructure.
- Bearer-token auth is sufficient for v1; mTLS can be added later without changing the high-level execution profile model.
