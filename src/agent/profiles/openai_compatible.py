from __future__ import annotations

from typing import Any

from ..tools import ToolRegistry
from .base import ProviderProfile
from .openai import build_openai_tool_registry


def create_openrouter_profile(*args: Any, **kwargs: Any) -> OpenAICompatibleProviderProfile:
    kwargs.setdefault("id", "openrouter")
    kwargs.setdefault("display_name", "OpenRouter")
    return OpenAICompatibleProviderProfile(*args, **kwargs)


def create_litellm_profile(*args: Any, **kwargs: Any) -> OpenAICompatibleProviderProfile:
    kwargs.setdefault("id", "litellm")
    kwargs.setdefault("display_name", "LiteLLM")
    return OpenAICompatibleProviderProfile(*args, **kwargs)


def build_openai_compatible_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return build_openai_tool_registry(provider_profile=provider_profile)


class OpenAICompatibleProviderProfile(ProviderProfile):
    def __post_init__(self) -> None:
        custom_tool_registry = self.tool_registry
        super().__post_init__()

        self.capabilities.setdefault("reasoning", False)
        self.supports_reasoning = False
        self.tool_registry = build_openai_compatible_tool_registry(provider_profile=self)
        if custom_tool_registry:
            if not isinstance(custom_tool_registry, ToolRegistry):
                custom_tool_registry = ToolRegistry(custom_tool_registry)
            for name, tool in custom_tool_registry.items():
                self.tool_registry.register(tool, name=name)


__all__ = [
    "OpenAICompatibleProviderProfile",
    "build_openai_compatible_tool_registry",
    "create_litellm_profile",
    "create_openrouter_profile",
]
