set shell := ["bash", "-lc"]

# Build and launch the desktop app with production frontend assets and release optimizations.
run-release: frontend-deps
  npm --prefix frontend run build
  cargo build --release -p spark-cli --bin spark
  cargo run --release -p spark-desktop --bin spark-desktop --all-features

# Repository validation gate: fmt check, all Rust tests, frontend unit tests, frontend build.
test: frontend-deps
  cargo fmt --all -- --check
  cargo test --workspace --all-features
  npm --prefix frontend run test:unit
  npm --prefix frontend run build

[private]
frontend-deps:
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest ]] || ! npm --prefix frontend ls --depth=0 >/dev/null 2>&1; then echo "Installing frontend dependencies with npm ci..." >&2; npm --prefix frontend ci; fi
