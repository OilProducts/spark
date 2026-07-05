# Rust Cutover Record

Spark now uses a Cargo-only distribution model. The supported install path is:

```bash
cargo install --path crates/spark-cli --bin spark
cargo install --path crates/spark-server --bin spark-server
```

The public command names remain `spark` and `spark-server`.

## Removed Boundaries

- Python runtime packages under `src/`
- Python wheel and sdist packaging metadata
- Python launcher compatibility wrappers
- pytest compatibility/oracle suites
- install recipes that create Python virtual environments

## Rust-Owned Assets

Runtime assets live under [crates/spark-assets/assets](../crates/spark-assets/assets):

- `flows/`
- `guides/`
- `unified_llm/data/models.json`

The frontend remains source-built with `npm --prefix frontend run build`. `spark-assets` embeds `frontend/dist` when present and uses its existing fallback only for development.

## Validation Gate

The cutover validation gate is:

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features
npm --prefix frontend run test:unit
npm --prefix frontend run build
```

Live provider smoke remains opt-in through Rust tests such as `crates/unified-llm-adapter/tests/live_smoke_contracts.rs` and is not part of default validation.
