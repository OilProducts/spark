# Cargo-Only Rust Cutover And Python Removal

## Summary

Convert the Rust rewrite worktree from "Rust runtime packaged through Python compatibility wrappers" to a hard Rust/Cargo distribution model. The supported install path becomes Cargo-built native binaries, not a Python wheel. Remove Python runtime, packaging, compatibility-oracle modules, and pytest-based validation once equivalent Rust/assets coverage exists.

## Key Changes

- Replace Python wheel distribution with Cargo-first installation:
  - Document source installs as:
    - `cargo install --path crates/spark-cli --bin spark`
    - `cargo install --path crates/spark-server --bin spark-server`
  - Keep public command names `spark` and `spark-server`.
  - Remove `pyproject.toml` console scripts, `src/spark/_rust_launcher.py`, wheel/sdist build scripts, `Dockerfile.wheel`, and `just install` paths that create `~/.spark/venv`.
- Move all runtime package data out of Python package directories:
  - Move authored flows and guides from `src/spark/...` into a Rust-owned asset tree under `crates/spark-assets/assets/...`.
  - Move `src/unified_llm/data/models.json` into the Rust asset tree.
  - Update `crates/spark-assets` to embed only files inside the Rust crate or workspace asset directories that are included by Cargo packaging.
  - Keep frontend assets Rust-owned: `npm --prefix frontend run build` remains the source build step; `spark-assets` embeds `frontend/dist` when present and uses the existing fallback only for development.
- Delete Python implementation and compatibility surfaces:
  - Remove `src/agent`, `src/attractor`, `src/spark`, `src/spark_common`, and `src/unified_llm` after assets are relocated.
  - Remove Python compatibility/oracle tests under `tests/agent`, `tests/adapters`, `tests/api`, `tests/compat`, and other pytest-only suites once their still-relevant behavior is covered by Rust crate tests.
  - Remove Python-specific fixtures only when no Rust test consumes them; preserve language-neutral JSON fixtures that are still useful by moving them under Rust test fixture directories.
- Update developer and release workflow:
  - Replace `uv sync`, `uv run pytest -q`, `uv build`, and wheel/sdist docs with Cargo and frontend commands.
  - Update `just setup`, `just test`, `just deliverable`, `just install`, and Docker/package docs to use Cargo-native binaries and no Python venv.
  - Make the default validation gate:
    - `cargo fmt --all -- --check`
    - `cargo test --workspace --all-features`
    - `npm --prefix frontend run test:unit`
    - `npm --prefix frontend run build`
  - Update `AGENTS.md` to remove the `uv run pytest` requirement after the cutover.

## Test Plan

- Rust command tests prove installed/source-built `spark` and `spark-server` expose the existing command surfaces without Python.
- Rust asset tests prove bundled flows, guides, model catalog, icons, and frontend index load from Rust-owned assets.
- Rust server tests prove `spark-server init`, `serve`, and `service install|status|remove` still work without `~/.spark/venv`.
- Rust workspace/API/SSE/agent tests cover the behavior previously protected by retained Python compatibility tests.
- Repo hygiene tests assert no tracked Python runtime/package/test entrypoints remain except explicitly allowed non-runtime helper scripts, if any.
- Final validation passes with the new Cargo/frontend gate and does not require `uv`, `pytest`, `pyproject.toml`, or Python package imports.

## Assumptions

- This is an intentionally breaking packaging change: `pip install spark` and `uv run spark` stop being supported for this Rust cutover.
- Cargo install is the primary distribution path for this pass; native tarballs, deb/rpm, Homebrew, and Tauri packaging are future packaging layers.
- Graphviz `dot` and Codex CLI remain external runtime prerequisites.
- Python may still exist on a developer machine, but Spark should not require it for build, install, runtime, or validation after this cutover.
