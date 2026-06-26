FROM python:3.11-slim AS deliverable-build

WORKDIR /src

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    SPARK_DELIVERABLE_OUT=/out \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

ENV PATH="${CARGO_HOME}/bin:${PATH}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg build-essential \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustc --version \
    && cargo --version \
    && pip install --no-cache-dir --upgrade pip uv \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/*

COPY . /src

RUN uv run python scripts/build_deliverable.py


FROM python:3.11-slim AS runtime

WORKDIR /app

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates curl git gnupg graphviz openssh-client ripgrep docker.io \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* \
    && pip install --no-cache-dir --upgrade pip

COPY --from=deliverable-build /out/spark-*.whl /tmp/spark-wheel/
RUN pip install --no-cache-dir /tmp/spark-wheel/spark-*.whl \
    && rm -rf /tmp/spark-wheel

COPY scripts/package-entrypoint.sh ./scripts/package-entrypoint.sh
RUN chmod 0755 ./scripts/package-entrypoint.sh

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    SPARK_HOME=/spark \
    SPARK_PROJECT_ROOTS=/projects \
    SPARK_DOCKER_HOME=/spark \
    ATTRACTOR_CODEX_RUNTIME_ROOT=/spark/runtime/codex \
    HOME=/spark/runtime/codex \
    CODEX_HOME=/spark/runtime/codex/.codex \
    XDG_CONFIG_HOME=/spark/runtime/codex/.config \
    XDG_DATA_HOME=/spark/runtime/codex/.local/share

EXPOSE 8000

CMD ["bash", "scripts/package-entrypoint.sh"]
