#!/bin/sh
# Builds spark-mathlab:latest from the repo root context; the worker binary
# is compiled inside Docker for the image architecture, so this works
# identically on Linux and macOS (Docker Desktop) hosts.
set -eu
cd "$(dirname "$0")/../.."
docker build -f containers/mathlab/Dockerfile -t spark-mathlab:latest .
