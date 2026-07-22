FROM rust:1.90-bookworm AS worker-builder
COPY . /src
RUN cargo build --release -p spark-server --manifest-path /src/Cargo.toml

FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git jq unzip zstd \
    build-essential cmake pkg-config \
    python3 python3-pip python3-venv \
    pari-gp nauty z3 cadical kissat drat-trim cvc5 \
    coinor-cbc coinor-csdp glpk-utils \
    nodejs npm \
    && rm -rf /var/lib/apt/lists/*

# Agent runtime: codex CLI. Codex auth is NOT baked into this image — the
# math-lab execution profile bind-mounts the host ~/.codex read-only at
# runtime via container.mounts and the entrypoint seeds it into $HOME.
RUN npm install -g @openai/codex

RUN git config --system safe.directory "*"

# Experimental-math Python stack.
RUN python3 -m venv /opt/mathlab/venv \
    && /opt/mathlab/venv/bin/pip install --no-cache-dir \
    sympy numpy scipy networkx matplotlib pandas \
    python-sat z3-solver cypari2 highspy
ENV PATH=/opt/mathlab/venv/bin:$PATH

# Lean 4 + pinned Mathlib with prebuilt cache.
ENV ELAN_HOME=/opt/elan
ENV PATH=/opt/elan/bin:$PATH
RUN curl -sSf https://elan.lean-lang.org/elan-init.sh | sh -s -- -y --default-toolchain none
RUN git clone --depth 1 https://github.com/leanprover-community/mathlib4 /opt/mathlab/mathlib4 \
    && cd /opt/mathlab/mathlib4 \
    && git rev-parse HEAD > /opt/mathlab/MATHLIB_COMMIT \
    && lake exe cache get \
    && lake build

# Workspace seed: flows copy this into empty math project directories.
COPY containers/mathlab/workspace-template/ /opt/mathlab/workspace-template/

# Prebuilt project template: agents copy this to start proving instantly.
COPY containers/mathlab/lean-template/ /opt/mathlab/template/
RUN cp /opt/mathlab/mathlib4/lean-toolchain /opt/mathlab/template/lean-toolchain \
    && cd /opt/mathlab/template && lake build

# Spark worker entrypoint, compiled inside Docker for the image's own
# architecture so the image builds identically on Linux and macOS hosts.
COPY --from=worker-builder /src/target/release/spark-server /usr/local/bin/spark-server

COPY containers/mathlab/mathlab-entry.sh /usr/local/bin/mathlab-entry.sh
RUN chmod +x /usr/local/bin/mathlab-entry.sh
ENTRYPOINT ["/usr/local/bin/mathlab-entry.sh"]
CMD ["tail", "-f", "/dev/null"]
