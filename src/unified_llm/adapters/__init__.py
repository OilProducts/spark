"""Placeholder provider adapters for the unified_llm package."""

# ruff: noqa: F401

from __future__ import annotations

import logging
from collections.abc import AsyncIterator
from typing import Any

from ..types import StreamEvent
from .base import ProviderAdapter, SupportsClose, SupportsInitialize, SupportsToolChoice

logger = logging.getLogger(__name__)


class _AdapterPlaceholder:
    name = "adapter"

    def __init__(self, **config: Any) -> None:
        self.config = dict(config)

    async def complete(self, request: Any) -> Any:
        logger.debug("%s.complete placeholder invoked", self.name)
        raise NotImplementedError(f"{self.__class__.__name__}.complete is not implemented")

    def stream(self, request: Any) -> AsyncIterator[StreamEvent]:
        logger.debug("%s.stream placeholder invoked", self.name)
        from ..streaming import StreamEventIterator

        return StreamEventIterator(adapter=self, request=request)

    def initialize(self) -> None:
        logger.debug("%s.initialize placeholder invoked", self.name)
        return None

    async def close(self) -> None:
        logger.debug("%s.close placeholder invoked", self.name)
        return None

    def supports_tool_choice(self, mode: str) -> bool:
        logger.debug("%s.supports_tool_choice placeholder invoked for mode=%s", self.name, mode)
        return False


class OpenAIAdapter(_AdapterPlaceholder):
    name = "openai"


class OpenAICompatibleAdapter(_AdapterPlaceholder):
    name = "openai_compatible"


class AnthropicAdapter(_AdapterPlaceholder):
    name = "anthropic"


class GeminiAdapter(_AdapterPlaceholder):
    name = "gemini"


__all__ = [
    "AnthropicAdapter",
    "GeminiAdapter",
    "OpenAIAdapter",
    "OpenAICompatibleAdapter",
    "ProviderAdapter",
    "SupportsClose",
    "SupportsInitialize",
    "SupportsToolChoice",
]
