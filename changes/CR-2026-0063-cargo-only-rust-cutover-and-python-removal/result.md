---
id: CR-2026-0063-cargo-only-rust-cutover-and-python-removal
title: Cargo-Only Rust Cutover And Python Removal
status: completed
type: feature
changelog: public
---

## Summary

Spark has been cut over to Cargo-built native `spark` and `spark-server` binaries. The Python runtime, Python package distribution, wheel build path, and pytest compatibility/oracle suites were removed after moving shipped assets and retained neutral fixtures into Rust-owned locations.

## Validation

The recorded full validation gate passed:

```bash
cargo fmt --all -- --check && cargo test --workspace --all-features && npm --prefix frontend run test:unit && npm --prefix frontend run build
```

Focused checks were also used while iterating across Rust API, CLI/server, asset, frontend, installed-command, and Codex app-server adapter contracts. Additional hygiene checks found no tracked `.py`, `pyproject.toml`, `MANIFEST.in`, `Dockerfile.wheel`, `requirements.txt`, or `uv.lock` files remaining in the maintained repository surfaces.

## Shipped Changes

- Removed tracked Python runtime/package code under `src/`, pytest suites under `tests/`, Python packaging files, wheel/sdist docs, and Python deliverable/compat scripts.
- Moved packaged flows, guides, and the unified LLM model catalog into `crates/spark-assets/assets/`, and moved retained language-neutral fixtures into `crates/test-fixtures/`.
- Updated `spark-assets`, Rust tests, source-checkout detection, service-install environment handling, installed binary root detection, Docker/compose, `justfile`, AGENTS guidance, README/docs/specs, and frontend unit coverage for the Cargo-only model.
- Kept the public command names `spark` and `spark-server`, with documented source installs through `cargo install --path crates/spark-cli --bin spark` and `cargo install --path crates/spark-server --bin spark-server`.
- Replaced Python-script Codex app-server test fakes with a native Rust JSON-RPC helper binary and added Rust coverage for Cargo-installed server initialization.
