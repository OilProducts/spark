FROM node:20-slim AS frontend-build

WORKDIR /frontend

COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

COPY frontend/ ./
RUN npm run build


FROM python:3.11-slim AS runtime

WORKDIR /app

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    DEBIAN_FRONTEND=noninteractive \
    SPARK_HOME=/spark \
    SPARK_PROJECT_ROOTS=/projects \
    ATTRACTOR_CODEX_RUNTIME_ROOT=/spark/runtime/codex

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git gnupg graphviz openssh-client ripgrep \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* \
    && pip install --no-cache-dir --upgrade pip

COPY pyproject.toml README-package.md ./
COPY src/ ./src/
COPY --from=frontend-build /frontend/dist ./src/spark/ui_dist/

RUN pip install --no-cache-dir /app

EXPOSE 8000

CMD ["spark-server", "serve", "--host", "0.0.0.0", "--port", "8000", "--data-dir", "/spark"]
