FROM rust:1-bookworm AS build

WORKDIR /src

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg build-essential \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/*

COPY . /src

RUN npm --prefix frontend ci
RUN npm --prefix frontend run build
RUN cargo build --release -p spark-cli --bin spark
RUN cargo build --release -p spark-server --bin spark-server

FROM debian:bookworm-slim AS runtime

WORKDIR /app

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates curl git gnupg graphviz openssh-client ripgrep docker.io \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/spark /usr/local/bin/spark
COPY --from=build /src/target/release/spark-server /usr/local/bin/spark-server
COPY scripts/package-entrypoint.sh ./scripts/package-entrypoint.sh

RUN chmod 0755 ./scripts/package-entrypoint.sh

ENV SPARK_HOME=/spark \
    SPARK_PROJECT_ROOTS=/projects \
    SPARK_DOCKER_HOME=/spark \
    ATTRACTOR_CODEX_RUNTIME_ROOT=/spark/runtime/codex \
    HOME=/spark/runtime/codex \
    CODEX_HOME=/spark/runtime/codex/.codex \
    XDG_CONFIG_HOME=/spark/runtime/codex/.config \
    XDG_DATA_HOME=/spark/runtime/codex/.local/share

EXPOSE 8000

CMD ["bash", "scripts/package-entrypoint.sh"]
