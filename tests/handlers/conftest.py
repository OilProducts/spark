from __future__ import annotations

import pytest

from attractor.handlers import build_default_registry
from tests.handlers._support.fakes import _StubBackend


@pytest.fixture
def stub_backend() -> _StubBackend:
    return _StubBackend()


@pytest.fixture
def default_registry(stub_backend: _StubBackend):
    return build_default_registry(stub_backend)
