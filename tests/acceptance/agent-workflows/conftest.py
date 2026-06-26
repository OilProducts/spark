from __future__ import annotations

import sys
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import harness


@pytest.fixture(autouse=True)
def _reset_acceptance_runtime(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    yield from harness.reset_acceptance_runtime(monkeypatch, tmp_path)


@pytest.fixture
def product_api_client() -> TestClient:
    with TestClient(harness.product_app.app) as client:
        yield client


@pytest.fixture
def attractor_api_client() -> TestClient:
    with TestClient(harness.attractor_server.attractor_app) as client:
        yield client
