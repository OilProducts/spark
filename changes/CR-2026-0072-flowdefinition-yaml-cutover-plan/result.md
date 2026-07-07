---
id: CR-2026-0072-flowdefinition-yaml-cutover-plan
title: FlowDefinition/YAML Cutover Plan
status: completed
type: feature
changelog: public
---

## Summary

Spark's authored workflow path has been cut over from DOT/Graphviz-centered flows to typed Rust `FlowDefinition` data with YAML as the first-class source format. The implementation adds the Rust model and schema artifact, loads and validates YAML flow sources, routes runtime execution from typed node and edge fields, persists typed flow snapshots, and updates API, CLI, workspace, frontend, bundled assets, docs, specs, and compatibility fixtures for `.yaml` flows.

The final implementation pass also closed review blockers around subflow input mapping and runtime handler dispatch: `NodeConfig::Subflow.input_map` is now applied when launching default child runs, and handler selection comes from `FlowNode.kind` instead of legacy extension overrides.

## Validation

Full validation gate completed:

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

Focused runtime coverage was added for default subflow input mapping and for ignoring legacy `extensions["type"]` handler overrides.

## Shipped Changes

- Added `FlowDefinition` and related typed flow model support in `attractor-core`, including YAML serde, semantic validation, normalization, and generated JSON Schema assets.
- Reworked flow source loading, path normalization, catalog behavior, API request handling, CLI flow commands, workspace routes, and run snapshot artifacts around `.yaml` / `.yml` flow sources and canonical flow-definition JSON.
- Updated runtime execution, routing, retry, terminal handling, context handling, manager-loop/subflow behavior, and storage surfaces to consume typed flow definitions instead of DOT graphs.
- Replaced frontend DOT canonical-model assumptions with FlowDefinition-shaped data for editor hydration, preview, save, validation, canvas rendering, graph settings, node authoring, launch controls, and related tests.
- Converted first-party bundled flows from `.dot` to `.yaml`, removed DOT authoring assets/tests/fixtures, and updated docs/specs/compatibility fixtures to describe the YAML FlowDefinition workflow model.
