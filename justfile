set shell := ["bash", "-lc"]

# Build and launch the desktop app with production frontend assets and release optimizations.
run-release: frontend-deps
  npm --prefix frontend run build
  cargo build --release -p spark-cli --bin spark
  if [[ "$(uname -s)" == Linux ]]; then just install-linux-desktop-entry; fi
  cargo run --release -p spark-desktop --bin spark-desktop --all-features

[private]
install-linux-desktop-entry:
  #!/usr/bin/env bash
  set -eu
  applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  mkdir -p "$applications"
  # GTK uses the executable name as the Wayland app ID.
  repo=${PWD//\\/\\\\}
  cat > "$applications/spark-desktop.desktop" <<EOF
  [Desktop Entry]
  Type=Application
  Name=Spark
  Exec=just run-release
  Path=$repo
  Icon=$repo/apps/spark-desktop/icons/icon.png
  EOF

# Repository validation gate: fmt check, all Rust tests, frontend unit tests, frontend build.
test: frontend-deps
  cargo fmt --all -- --check
  ulimit -Sn 4096 && cargo test --workspace --all-features
  npm --prefix frontend run test:unit
  npm --prefix frontend run build

[private]
frontend-deps:
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest ]] || ! npm --prefix frontend ls --depth=0 >/dev/null 2>&1; then echo "Installing frontend dependencies with npm ci..." >&2; npm --prefix frontend ci; fi
