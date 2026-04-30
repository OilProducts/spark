# Run-Scoped Execution Containers for Flow Runs

## Summary

Add a second execution mode for workflow runs:

- **Native mode:** current behavior. Spark runs the flow in the same environment as the Spark server.
- **Container mode:** Spark remains the control plane, but a project default or run override declares an execution image. Spark starts one run-scoped container and executes **all flow node handlers** inside that container.

V1 is workflow-run only. Project chat command execution remains unchanged until this same execution profile is extended there later.

## Public Interfaces

- Add an execution container field to project metadata:
  - optional `execution_container_image`
  - absent/empty means native execution.
- Add run-level override support through:
  - workspace run launch requests
  - conversation flow-run requests
  - Attractor `PipelineStartRequest`
  - `spark run launch --execution-container <image>`
- Resolution order:
  1. explicit run launch image
  2. project default image
  3. native mode
- Persist selected execution mode/image in run metadata and seed it into run context for observability.
- Do not add flow-DOT execution-container semantics in v1.

## Implementation Changes

- Keep `PipelineExecutor` in the Spark control plane for routing, checkpoints, run records, cancellation state, and UI event publication.
- Add a `ContainerizedHandlerRunner` that implements the same runner contract as `HandlerRunner`, but delegates node handler execution into a run container.
- Add a hidden worker command, for example `spark-server worker run-node`, available inside Spark-capable images:
  - reads a JSON node-execution request from stdin
  - builds the normal handler registry inside the container
  - executes the requested handler with container-local paths
  - streams JSONL events back to the control plane
  - returns a serialized `Outcome`
- The worker must execute all handler types in container mode: `start`, `exit`, `codergen`, `tool`, `conditional`, `parallel`, `parallel.fan_in`, `wait.human`, and `stack.manager_loop`.
- Use one run-scoped container:
  - create/start before first node
  - `docker exec -i ... spark-server worker run-node` per node
  - stop/remove when the run reaches a terminal state or is canceled
  - label containers with run/project metadata for cleanup/debugging
- For packaged Docker, add the operational prerequisites:
  - packaged Spark image includes Docker CLI
  - compose mounts `/var/run/docker.sock`
  - compose passes `SPARK_DOCKER_HOME` and `SPARK_PROJECTS_HOST_DIR`
  - path mapper translates control-plane paths like `/projects/...` and `/spark/...` back to host bind paths before creating sibling execution containers
- Standard mounts/env for v1:
  - project working directory mounted into the execution container
  - Spark run root mounted so logs/artifacts remain visible to the control plane
  - Spark runtime/Codex auth material mounted at the same Spark runtime path
  - provider env vars passed through using the existing provider allowlist
- Human gates and child runs stay control-plane coordinated:
  - worker emits human-gate requests over the worker protocol; control plane uses the existing broker and replies
  - manager child runs launch through the control plane but reuse the same root run container
  - child status lookup remains control-plane owned

## Failure Behavior

- If container mode is selected and Docker is unavailable, fail before node execution with a clear validation/setup error.
- If required host-path mapping is unavailable in packaged Docker, fail before node execution with a configuration error.
- If the image is missing or cannot start, fail the run setup clearly; do not silently fall back to native mode.
- Canceling a containerized run terminates the active `docker exec`; if needed, stop/remove the run container.

## Test Plan

- Unit test execution-mode resolution: run override, project default, native fallback.
- API/CLI tests for carrying `execution_container_image` through launch requests and run metadata.
- Fake-Docker tests for container creation, path mapping, env/mount construction, cleanup, and no native fallback.
- Worker protocol tests for event streaming, outcome serialization, human-gate request/reply, and child-run delegation.
- Behavioral flow tests with fake container transport proving a `tool` node and an LLM-backed node both execute through the same container runner.
- Keep live Docker coverage optional/marked; the normal suite should not require a real Docker daemon.

## Assumptions

- V1 execution image must already be Spark-capable: it contains the installed Spark package, `spark-server worker`, Codex/runtime dependencies, and project tools such as `uv`.
- V1 exposes image-only configuration. Custom mounts, env allowlists, Docker args, image build, and image pull policy are deferred.
- Flow DOT remains portable and does not declare containers in this first version.
