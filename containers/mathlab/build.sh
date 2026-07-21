#!/bin/sh
# Builds the spark-mathlab execution image. Run from the repo root or this
# directory; stages the release spark-server worker binary into the build
# context first.
set -eu
cd "$(dirname "$0")"
cargo build --release -p spark-server --manifest-path ../../Cargo.toml
cp ../../target/release/spark-server spark-server-bin
docker build -t spark-mathlab:latest .
