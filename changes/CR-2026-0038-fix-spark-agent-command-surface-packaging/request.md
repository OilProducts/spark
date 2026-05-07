# Fix Spark Agent Command Surface Packaging

## Summary
The main break is packaging/runtime initialization, not the agent CLI shape. Packaged flows are seeded into `SPARK_HOME/flows`, but no `flow-catalog.toml` is seeded, so every flow defaults to `disabled`. Since the agent CLI always queries `surface=agent`, `spark flow list` returns `[]` and `describe/get` 404 for flows that exist but are not agent-requestable.

Keep the source-checkout guardrails as-is: humans in a checkout should use `uv run spark ...` with explicit `SPARK_API_BASE_URL`; installed/package/runtime assistant surfaces should continue using bare `spark`.

## Key Changes
- Seed default launch-policy catalog entries during runtime initialization.
  - Add a small default agent-requestable set:
    - `software-development/implement-change-request.dot`
    - `software-development/spec-implementation/implement-spec.dot`
  - Keep examples and child/internal flows disabled by default, including `implement-milestone.dot`.
  - Preserve existing operator choices: only write missing catalog entries; do not overwrite an existing `agent_requestable`, `trigger_only`, or `disabled` entry.

- Wire the catalog seeding through the existing initialization path.
  - `spark-server init`
  - `spark-server service install`
  - packaged Docker startup via `package-entrypoint.sh`, because it already runs `spark-server init --data-dir /spark`
  - `just install` and `just run-docker` inherit the fix through those paths.

- Keep CLI filtering semantics unchanged.
  - `spark flow list` remains agent-only.
  - `spark flow describe/get` remain agent-only.
  - The fix is that the packaged/core flows are actually agent-requestable after init.

- Optionally improve the empty-list text output.
  - For `spark flow list --text`, print a short “No agent-requestable flows found” message when the agent surface is empty.
  - Do not change JSON output; keep `[]` for scripts.

## Test Plan
- Add/adjust `spark-server init` tests to assert it creates `config/flow-catalog.toml` with the two core flow entries set to `agent_requestable`.
- Add a preservation test: if `flow-catalog.toml` already disables one of those flows, `spark-server init` does not overwrite it.
- Add an API/CLI-level behavior test showing seeded core flows appear through `surface=agent`.
- Keep existing tests that uncataloged flows default to `disabled`.
- Run `uv run pytest -q`.

## Assumptions
- “Core only” means the two user-facing software-development flows, not examples and not child worker flows.
- This fix should not make source-checkout `spark` magically available on a human shell `PATH`; that remains an install/environment concern.
- The assistant runtime should keep using bare `spark` when Spark is installed or packaged, because the runtime environment already adds the active Python environment’s bin directory to Codex’s `PATH`.
