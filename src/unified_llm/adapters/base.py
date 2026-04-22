from __future__ import annotations

import logging
from collections.abc import AsyncIterator
from typing import Any, Protocol, runtime_checkable

from ..types import Request, Response, StreamEvent

logger = logging.getLogger(__name__)


@runtime_checkable
class ProviderAdapter(Protocol):
    name: str

    async def complete(self, request: Request) -> Response: ...

    def stream(self, request: Request) -> AsyncIterator[StreamEvent]: ...


@runtime_checkable
class SupportsInitialize(Protocol):
    def initialize(self) -> Any: ...


@runtime_checkable
class SupportsClose(Protocol):
    def close(self) -> Any: ...


@runtime_checkable
class SupportsToolChoice(Protocol):
    def supports_tool_choice(self, mode: str) -> bool: ...


__all__ = [
    "ProviderAdapter",
    "SupportsClose",
    "SupportsInitialize",
    "SupportsToolChoice",
]
