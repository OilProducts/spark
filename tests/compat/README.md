# Compatibility Harness

M0 compatibility tests capture and replay observable Python behavior before any
Rust replacement retires that behavior. Later fixture items should use
`scripts/compat_capture.py` and `tests.compat.harness` instead of inventing a
new manifest or assertion format.

## Layout

- `.spark/rust-rewrite/current/compat-fixtures/` holds intentionally reviewed
  golden fixture manifests and payload snapshots.
- `.spark/rust-rewrite/current/validation/` holds generated captures,
  temporary Spark homes, command logs, caches, package artifacts, and frontend
  build output used during local validation.
- `tests/compat/_generated/` and `tests/compat/.tmp/` are scratch locations for
  compatibility tests and are ignored.

Golden fixture names should include the domain and scenario, for example
`cli/flow-validate-success` or `http/workspace-project-not-found`. Generated
captures should stay under `validation/generated/` unless a later item promotes
them into reviewed fixtures.

## CLI And Filesystem Fixtures

M0-I02 adds reviewed CLI fixtures under `compat-fixtures/cli/` and durable
filesystem fixtures under `compat-fixtures/filesystem/`. Local CLI fixtures run
real `uv run spark` and `uv run spark-server` commands against temporary
`SPARK_HOME`, flow, runtime, and Codex homes. Server-backed CLI fixtures start a
local Python `spark-server serve` process on an ephemeral localhost port and do
not require provider credentials, external network access, installed assets, or
package build output.

Fixture assertions compare process exit status, stdout, stderr, normalized
command argv, filesystem snapshots, and parsed JSON/TOML/JSONL state. Dynamic
timestamps, trigger ids, webhook credentials, conversation handles, project ids,
temporary paths, and ephemeral localhost ports are normalized before comparison.

## HTTP And SSE Fixtures

M0-I03 adds reviewed HTTP route fixtures under `compat-fixtures/http/` and
app-shell live stream fixtures under `compat-fixtures/sse/`. These tests start
a real local Python `spark-server serve` process from the isolated rewrite
worktree and issue requests through `httpx`; they do not inspect source,
private route objects, prompt text, docs, external provider credentials, package
build output, or installed bundled assets.

HTTP manifests record method, path, query, request body, status code, selected
user-visible headers, response body kind, server provenance, and
requirement/decision coverage. SSE manifests record stream request parameters,
selected stream headers, bounded event/comment frames, decoded JSON envelopes,
cursor/resource fields, and representative conversation, trigger, deprecated,
guard, and keepalive behavior.

Dynamic localhost ports, temporary Spark homes and project paths, generated
trigger ids, webhook keys/secrets, timestamps, run ids, file sizes, and hashes
are normalized by `tests.compat.harness` before golden comparison. Source
development UI/static route behavior is included here only at the HTTP
boundary; frontend contract payloads, frontend unit checks, packaging smoke
output, and installed bundled asset parity remain assigned to M0-I05 and later
packaging milestones.

## Frontend Contract And Packaging Fixtures

M0-I05 adds frontend contract fixtures under `compat-fixtures/frontend/` and
packaging smoke fixtures under `compat-fixtures/packaging/`. The frontend
fixtures run the existing behavior-contract Vitest file and compact TypeScript
probes through public parser, request builder, API error, canonical flow model,
and live-event URL helpers. They do not inspect frontend source text.

Packaging fixtures capture source/development `SPARK_UI_DIR` static serving,
deliverable build output, installed command init behavior, public
`spark-server service install|status|remove` behavior through a temporary
`XDG_CONFIG_HOME` and fake `systemctl`, and package resource presence. Build
output, virtual environments, wheelhouses, source UI scratch directories, and
service smoke data stay in pytest temp directories or validation generated
roots. M0-I05 records source and smoke evidence only; installed bundled asset
parity remains an M6 gate.

## Commands

Focused harness validation:

```bash
uv run pytest -q tests/compat -k harness
```

First-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat -k harness
```

Full Python guardrail before reporting a completed code change:

```bash
uv run pytest -q
```

Focused M0-I02 validation:

```bash
uv run pytest -q tests/compat/cli tests/compat/storage
```

M0-I02 first-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat/cli tests/compat/storage
```

Focused M0-I03 validation:

```bash
uv run pytest -q tests/compat/api tests/compat/live
```

M0-I03 first-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat/api tests/compat/live
```

Manual M0-I03 fixture regeneration:

```bash
uv run pytest -q tests/compat/api tests/compat/live --compat-update-goldens
```

Focused M0-I05 validation:

```bash
uv run pytest -q tests/contracts/frontend tests/compat/frontend-contracts tests/compat/packaging
npm --prefix frontend run test:unit
```

M0-I05 first-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat/packaging
```

Manual M0-I05 fixture regeneration:

```bash
uv run pytest -q tests/compat/frontend-contracts tests/compat/packaging --compat-update-goldens
```

## M0 Coverage Ledger And Validation Gate

M0-I06 adds the milestone coverage ledger at
`.spark/rust-rewrite/current/validation/m0-coverage-ledger.json` and the gate
record at `.spark/rust-rewrite/current/validation/m0-validation-gate.json`.
The ledger maps M0 fixture groups, requirement ids, contract-decision ids,
representative fixture ids, validation suites, explicit gaps, and closure
evidence. The gate record names the required guardrail commands, first-failure
triage commands, future Rust command expectations, generated-artifact hygiene
rules, and the no-retired-Python closure statement.

Focused M0-I06 validation:

```bash
uv run pytest -q tests/compat/test_m0_coverage_validation_gate.py
uv run pytest -q tests/compat
```

M0-I06 first-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat/test_m0_coverage_validation_gate.py
uv run pytest -q -x --maxfail=1 tests/compat
```

Milestone guardrails:

```bash
uv run pytest -q
npm --prefix frontend run test:unit
```

Future Rust gates recorded by M0-I06:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
```

The M0 gate is evidence for the compatibility oracle only. Later Rust
milestones must pass the relevant fixture groups before retiring Python
behavior; installed bundled asset closure and production trigger runtime remain
later gates.

## DSL, Runtime, Handler, And Journal Fixtures

M0-I04 adds reviewed DSL fixtures under `compat-fixtures/dsl/` and runtime,
handler, API-journal, durable-state, and execution-profile fixtures under
`compat-fixtures/runtime/`. These tests use public parser, formatter,
validator, preview, transform, `PipelineExecutor`, `HandlerRunner`, execution
profile, real local HTTP API, and durable run-file interfaces. Fake LLM,
interviewer, custom handler, and child-run backends are deterministic local
test doubles so fixture capture does not require provider credentials, Docker,
external network services, or human input.

Runtime manifests normalize run roots, temporary project paths, timestamps,
run ids, event ids, hashes, file sizes, retry delay jitter, and generated
child-run ids. They keep the observable graph payloads, canonical DOT,
diagnostics, route traces, outcomes, context, checkpoint state, journal entries,
API payloads, handler outputs, artifact files, and selected durable JSON/JSONL
records as the parity oracle. Generated run directories remain in pytest temp
locations or `validation/generated/`; only compact reviewed manifests are
promoted to `compat-fixtures/dsl/` and `compat-fixtures/runtime/`.

Focused M0-I04 validation:

```bash
uv run pytest -q tests/compat/dsl tests/compat/transforms tests/compat/runtime tests/compat/handlers tests/compat/execution
```

M0-I04 first-failure triage:

```bash
uv run pytest -q -x --maxfail=1 tests/compat/dsl tests/compat/transforms tests/compat/runtime tests/compat/handlers tests/compat/execution
```

Manual M0-I04 fixture regeneration:

```bash
uv run pytest -q tests/compat/dsl tests/compat/transforms tests/compat/runtime tests/compat/handlers tests/compat/execution --compat-update-goldens
```
