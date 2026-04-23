from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from ...models import get_model_info
from ..builtin_tools import build_openai_builtin_tool_registry, register_openai_builtin_tools
from ..subagents import register_subagent_tools
from ..tools import ToolRegistry
from .base import ProviderProfile


def create_openai_profile(*args: Any, **kwargs: Any) -> OpenAIProviderProfile:
    return OpenAIProviderProfile(*args, **kwargs)


def register_openai_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    register_openai_builtin_tools(
        target_registry,
        provider_profile=provider_profile,
    )
    register_subagent_tools(target_registry, provider_profile=provider_profile)
    return target_registry


def build_openai_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    registry = build_openai_builtin_tool_registry(provider_profile=provider_profile)
    return register_subagent_tools(registry, provider_profile=provider_profile)


class OpenAIProviderProfile(ProviderProfile):
    def __post_init__(self) -> None:
        custom_tool_registry = self.tool_registry
        super().__post_init__()

        if not self.id:
            self.id = "openai"

        model_info = (
            get_model_info(self.model)
            if isinstance(self.model, str) and self.model
            else None
        )
        if model_info is not None:
            if self.display_name is None:
                self.display_name = model_info.display_name
            if self.context_window_size is None:
                self.context_window_size = model_info.context_window
            self.capabilities.setdefault("reasoning", model_info.supports_reasoning)
            self.capabilities.setdefault("vision", model_info.supports_vision)
            self.supports_reasoning = bool(
                self.supports_reasoning or self.capabilities.get("reasoning")
            )
        elif self.display_name is None and not self.model:
            self.display_name = "OpenAI"

        self.tool_registry = build_openai_tool_registry(provider_profile=self)
        if custom_tool_registry:
            if not isinstance(custom_tool_registry, ToolRegistry):
                custom_tool_registry = ToolRegistry(custom_tool_registry)
            for name, tool in custom_tool_registry.items():
                self.tool_registry.register(tool, name=name)

    def provider_options(self, session_config: Any | None = None) -> dict[str, Any]:
        options = super().provider_options()
        if session_config is None:
            return options

        reasoning_effort = getattr(session_config, "reasoning_effort", None)
        if reasoning_effort is None:
            return options

        reasoning_options: dict[str, Any] = {}
        existing_reasoning = options.get("reasoning")
        if isinstance(existing_reasoning, Mapping):
            reasoning_options.update(existing_reasoning)
        reasoning_options["effort"] = reasoning_effort
        options["reasoning"] = reasoning_options
        return options


__all__ = [
    "OpenAIProviderProfile",
    "create_openai_profile",
    "build_openai_tool_registry",
    "register_openai_tools",
]
