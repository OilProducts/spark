# FlowDefinition/YAML Cutover Plan

## Summary

Replace Spark’s DOT-centered workflow model with a typed Rust `FlowDefinition` model, YAML as the primary authored workflow format, and Rust-generated JSON Schema as the public validation contract. This is a hard cutover for first-party/new Rust Spark behavior: bundled flows move from `.dot` to `.yaml`, runtime/preview/API consume `FlowDefinition`, and DOT parser/formatter behavior is removed from the main flow path rather than kept as a compatibility adapter.

## Key Changes

- Add `FlowDefinition` types in `attractor-core` as the semantic source of truth:
  - `FlowDefinition { schema_version, id, title, description, goal, inputs, defaults, nodes, edges, metadata, extensions }`
  - `FlowNode { kind, label, description, config, context, retry, execution, ui, extensions }`
  - `FlowEdge { from, to, label, condition, weight, transition }`
  - Tagged `NodeConfig` variants for `start`, `exit`, `agent_task`, `human_gate`, `conditional`, `parallel`, `fan_in`, `tool`, and `subflow`.
- Add `schemars` to generate `flow-definition.schema.json` from Rust types; commit the generated schema under project assets for frontend/editor/docs consumption.
- Add YAML parsing via `serde_yaml`; parse YAML into `FlowDefinition`, validate structurally and semantically, then normalize before runtime use.
- Replace DOT-specific runtime input/output with flow-definition equivalents:
  - `ExecuteRunRequest.graph: DotGraph` becomes `flow: FlowDefinition`.
  - runtime routing resolves start/exit, outgoing edges, conditional nodes, retry policy, goal gates, handler type, prompts, context contracts, and model settings from typed fields.
  - run snapshots persist canonical `flow-definition.json` and original authored `flow-source.yaml`.
- Replace Graphviz preview as the primary editor/runtime graph payload with a `FlowDefinition` preview payload:
  - keep existing preview response concepts: nodes, edges, diagnostics, child previews, graph metadata.
  - remove `rawDot`, `graph_attrs`, DOT defaults/subgraphs from canonical frontend model.
  - frontend canvas serializes back to YAML/FlowDefinition-shaped data, not DOT.
- Convert bundled flows in `crates/spark-assets/assets/flows` from `.dot` to `.yaml`; update flow names, trigger fixtures, run launch fixtures, docs, and guides to use `.yaml`.
- Change flow path normalization to accept `.yaml` and `.yml`; stop auto-appending `.dot`. If no extension is supplied, append `.yaml`.
- Update CLI/API behavior:
  - `spark flow list/get/describe/validate` operates on YAML flow files.
  - `spark flow validate --file` validates YAML/FlowDefinition against generated schema plus semantic rules.
  - remove or repurpose DOT-only `spark flow format`; if kept, it formats YAML only.
  - `PipelineStartRequest.flow_content` is treated as YAML source; API stores canonical flow snapshot JSON.
- Update workspace flow summaries to read typed metadata:
  - title from `title`, description from `description`, goal from `goal`.
  - features from `FlowNode.kind`, not shape/type attributes.
  - launch policy catalog remains keyed by normalized flow name.

## Implementation Plan

- Core model and validation:
  - Add `flow_definition.rs` to `attractor-core`, export it from `lib.rs`, and derive `Serialize`, `Deserialize`, and `JsonSchema`.
  - Implement `FlowDefinition::validate()` for required invariants: non-empty id, valid node ids, exactly one start and exit, valid edges, reachability, no outgoing edges from exit, context contract key syntax, valid subflow references, valid human decisions, and parseable conditions.
  - Implement `FlowDefinition::normalize()` for deterministic ordering-sensitive output, default filling, and stable snapshot JSON.
- Source loading:
  - Replace `attractor-dsl::flow_sources` DOT assumptions with format-neutral helpers: normalize names, resolve `.yaml/.yml`, read source, parse YAML, return `NamedFlowSource { name, path, content, flow }`.
  - Remove DOT semantic-equivalence checks from save; for YAML saves, validate parsed canonical flow before writing.
  - Add schema generation command/test helper that writes the schema artifact when intentionally regenerated.
- Runtime:
  - Update executor, routing, retry, terminal, context, handlers, manager-loop, and storage APIs to accept `FlowDefinition`, `FlowNode`, and `FlowEdge`.
  - Replace handler resolution from `shape/type` with `FlowNode.kind`.
  - Replace ad hoc attr lookups with typed config accessors; keep `extensions` only for non-core/experimental metadata.
  - Persist run artifacts as:
    - `artifacts/flow/flow-source.yaml`
    - `artifacts/flow/flow-definition.json`
    - no required `graphviz/pipeline.dot` artifact.
- API/workspace:
  - Update `PreviewRequest` to carry `flow_content` YAML and optional `flow_name`.
  - Update start/continue/retry paths to load/snapshot `FlowDefinition`.
  - Update `/workspace/flows` summaries/descriptions/validation to use typed flow metadata.
  - Update child/subflow resolution to use `NodeConfig::Subflow { flow_ref, input_map }`.
- Frontend:
  - Replace DOT canonical model types with FlowDefinition types generated or mirrored from JSON Schema.
  - Update flow tree, editor, launch input panels, graph settings, canvas hydration, save, preview, and validation to use typed fields.
  - Remove DOT graph-attribute UI and shape-to-handler assumptions; expose node kind/config controls instead.
- Assets/docs:
  - Convert first-party flows to YAML.
  - Replace `dot-authoring.md` with a YAML/FlowDefinition authoring guide.
  - Update `spark-operations.md`, specs, and compatibility fixtures to refer to YAML flow names.

## Test Plan

- Rust unit/contract tests:
  - FlowDefinition serde round-trip from YAML and canonical JSON.
  - generated JSON Schema validates representative YAML-parsed JSON.
  - semantic validation failures for missing start/exit, invalid edges, unreachable nodes, invalid context keys, invalid human gate decisions, and invalid conditions.
  - runtime executes simple linear, branch, human gate, parallel, manager/subflow, retry, and continue flows from YAML.
  - run storage writes `flow-source.yaml` and `flow-definition.json`.
  - workspace catalog lists/describes/validates `.yaml` flows and rejects unknown/invalid extensions.
  - CLI `flow validate --file` succeeds/fails with updated text and JSON output.
- Frontend tests:
  - preview parsing and canvas hydration from FlowDefinition payload.
  - node kind rendering replaces shape rendering.
  - editor save/validate payloads use YAML/FlowDefinition semantics.
  - launch input drafts still work from typed `inputs`.
- Fixture migration:
  - Replace DOT compatibility fixtures that represent current intended behavior with YAML equivalents.
  - Delete tests that assert DOT syntax, DOT formatting, Graphviz shape mappings, or `rawDot` behavior.
- Full validation gate before completion:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions

- This is a breaking hard cutover for new Rust Spark: DOT is not preserved as a supported authoring/runtime path in the main implementation.
- Rust `FlowDefinition` types are the semantic source of truth; JSON Schema is generated from Rust and checked into the repo.
- YAML is the only first-class hand-authored workflow format for v1 of the new model.
- Canonical JSON is used for APIs, run snapshots, frontend validation, and schema tooling, but users are not expected to hand-author JSON flows.
- Existing crate names may remain initially to keep the diff bounded; renaming `attractor-*` crates is a separate cleanup after the workflow model cutover.
