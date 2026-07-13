set shell := ["bash", "-lc"]

[private]
frontend-deps:
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest ]]; then echo "Installing frontend dependencies with npm ci..." >&2; npm --prefix frontend ci; fi

# Developer setup for the Cargo-backed source checkout and frontend.
setup:
  npm --prefix frontend ci
  cargo fetch

dev-docker:
  bash scripts/dev-docker.sh

run-docker:
  bash scripts/run-docker.sh

dev-run: frontend-deps
  bash scripts/dev-run.sh

# Launch the Tauri desktop app (self-seeding runtime) with a fresh frontend build.
dev-desktop: frontend-deps
  npm --prefix frontend run build
  cargo run -p spark-desktop --bin spark-desktop --all-features

# Build and launch the desktop app with production frontend assets and release optimizations.
run-release: frontend-deps
  npm --prefix frontend run build
  cargo run --release -p spark-desktop --bin spark-desktop --all-features

# Repository validation gate for the Rust cutover.
test: frontend-deps
  cargo fmt --all -- --check
  cargo test --workspace --all-features
  npm --prefix frontend run test:unit
  npm --prefix frontend run build

# Report Cargo target usage without changing cached artifacts.
rust-cache-size:
  target_dir="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; echo "Cargo target: $target_dir"; if [[ -d "$target_dir" ]]; then du -sh "$target_dir"; find "$target_dir" -mindepth 1 -maxdepth 1 -type d -print0 | xargs -0 -r du -sh | sort -hr; else echo "Target directory does not exist."; fi

# Explicitly remove only Cargo's configured target directory.
clean-rust-cache:
  target_dir="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; echo "Removing Cargo target: $target_dir"; cargo clean

# Build local release binaries and the production frontend.
deliverable: frontend-deps
  npm --prefix frontend run build
  cargo build --workspace --release

# Install the public command names from this source checkout.
install:
  cargo install --path crates/spark-cli --bin spark
  cargo install --path crates/spark-server --bin spark-server
  spark-server init

install-systemd:
  cargo install --path crates/spark-cli --bin spark
  cargo install --path crates/spark-server --bin spark-server
  spark-server service install --host "${SPARK_HOST:-0.0.0.0}" --port "${SPARK_PORT:-8000}" --data-dir "${SPARK_HOME:-$HOME/.spark}"
