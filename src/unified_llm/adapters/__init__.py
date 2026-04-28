"""Provider adapters for the unified_llm package."""

from __future__ import annotations

from typing import Any

from .base import ProviderAdapter, SupportsClose, SupportsInitialize, SupportsToolChoice


def __getattr__(name: str) -> Any:
    if name == "AnthropicAdapter":
        from .anthropic import AnthropicAdapter as _AnthropicAdapter

        globals()["AnthropicAdapter"] = _AnthropicAdapter
        return _AnthropicAdapter

    if name == "OpenAIAdapter":
        from .openai import OpenAIAdapter as _OpenAIAdapter

        globals()["OpenAIAdapter"] = _OpenAIAdapter
        return _OpenAIAdapter

    if name == "OpenAICompatibleAdapter":
        from .openai_compatible import OpenAICompatibleAdapter as _OpenAICompatibleAdapter

        globals()["OpenAICompatibleAdapter"] = _OpenAICompatibleAdapter
        return _OpenAICompatibleAdapter

    if name == "OpenRouterAdapter":
        from .openai_compatible import OpenRouterAdapter as _OpenRouterAdapter

        globals()["OpenRouterAdapter"] = _OpenRouterAdapter
        return _OpenRouterAdapter

    if name == "LiteLLMAdapter":
        from .openai_compatible import LiteLLMAdapter as _LiteLLMAdapter

        globals()["LiteLLMAdapter"] = _LiteLLMAdapter
        return _LiteLLMAdapter

    if name == "GeminiAdapter":
        from .gemini import GeminiAdapter as _GeminiAdapter

        globals()["GeminiAdapter"] = _GeminiAdapter
        return _GeminiAdapter

    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


__all__ = [
    "AnthropicAdapter",
    "GeminiAdapter",
    "OpenAIAdapter",
    "OpenAICompatibleAdapter",
    "OpenRouterAdapter",
    "LiteLLMAdapter",
    "ProviderAdapter",
    "SupportsClose",
    "SupportsInitialize",
    "SupportsToolChoice",
]
