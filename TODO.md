# TODO

## Known issues

- [ ] **Human steering cannot reach a node running in a container execution
  profile.** `steer_pipeline_route` delivers into native Claude and Codex
  node turns as a live message (CR-2026-0151) but rejects container runs
  with `container_execution_unsupported`: the agent session lives in the
  worker, which has no channel for live messages. A steer sent while no
  agent turn runs is rejected, not queued.

- [ ] **Agent tool processes share the desktop's process scope; a runaway
  child can OOM the whole product.** 2026-07-22: a loam validation test
  (`loam_server-*` integration binary, spawned by a milestone child's
  `cargo test --workspace`) allocated 116 GB, the kernel OOM killer fired
  inside the hosting terminal scope, and the scope teardown SIGKILLed
  spark-desktop along with it — GNOME then blamed "Spark Desktop" for the
  memory use. The desktop itself was at ~3.2 GB. Fix directions: run
  node tool/agent commands in their own transient systemd scope with
  MemoryHigh/MemoryMax (native execution), and/or lean on container
  execution profiles whose cgroups already provide the boundary; either
  way a child's death must be a node failure, not a product crash.
- [ ] **Playwright smoke specs** predate the runs-tab overhaul; re-record the runs
  smoke flows against the Activity stream / inspector layout. Confirmed stale
  2026-07-21: `runs-observability.spec.ts` asserts `run-advanced-panel` and
  `run-advanced-toggle-button`, which no longer exist anywhere in
  `frontend/src`.
- [ ] **Manual desktop verification of blocking human gates**
  (`just dev-desktop`). Narrowed 2026-07-21: live chat streaming and live run
  streaming were verified extensively in production desktop use during the
  2026-07-19..21 loam build (conversation nimble-stream, multi-hour milestone
  runs); blocking `human_gate` nodes were never exercised on the desktop in
  that period and still need a manual pass.
- [ ] **Parent-run usage rollup**: run token usage/cost is per-record; child-run
  usage does not aggregate into the parent (matches old Python behavior). Roll
  up over the combined journal if parent totals should include children.

## Done

- [x] **Run event logs: metadata-only reads and incremental journal tailing**
  (implemented 2026-07-21, branch `spark/live-run-perf-fixes`, commits
  `a3357ec`/`9eb385d`/`f045773`). RunMeta reads records/checkpoints without
  touching events.jsonl (runs list, control routes, record updates); the
  manager observes children from metadata plus a tail-read; the human-gate
  wait loop scans only appended bytes per poll; and the publisher/cursor
  replay are served by a bounded per-run incremental cache (byte offsets +
  recent-entry ring + incremental segment projection) that contract tests
  hold byte-equal to the cold rebuild, including the out-of-order-append
  rebuild path. Cold paths (transcript, deep page-back, raw events)
  intentionally still rebuild from disk. Live verification 2026-07-21:
  after 4h18m of continuous milestone streaming the desktop held ~3.2 GB
  RSS at 3.5% CPU, versus 26.6 GB overnight growth on the prior binary —
  RSS stays flat.

- [x] Build an executable harness for the existing agent-driven acceptance workflow assets under `tests/acceptance/agent-workflows`.
- [x] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [x] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.
