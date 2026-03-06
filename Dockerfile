FROM python:3.11-slim

WORKDIR /app

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates git gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g @openai/codex \
    && apt-get purge -y --auto-remove curl gnupg \
    && rm -rf /var/lib/apt/lists/* \
    && pip install --no-cache-dir --upgrade pip

COPY pyproject.toml /app/pyproject.toml
COPY attractor /app/attractor
RUN pip install --no-cache-dir -e /app
