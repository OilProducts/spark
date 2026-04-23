"""Public provider profile extensibility surface for unified_llm.agent."""

from __future__ import annotations

from typing import Any

from .base import ProviderProfile

__all__ = [
    "AnthropicProviderProfile",
    "build_anthropic_tool_registry",
    "GeminiProviderProfile",
    "build_gemini_tool_registry",
    "OpenAIProviderProfile",
    "ProviderProfile",
    "create_anthropic_profile",
    "create_gemini_profile",
    "create_openai_profile",
    "register_anthropic_tools",
    "register_gemini_tools",
    "build_openai_tool_registry",
    "register_openai_tools",
]


def __getattr__(name: str) -> Any:
    if name not in {
        "AnthropicProviderProfile",
        "build_anthropic_tool_registry",
        "GeminiProviderProfile",
        "build_gemini_tool_registry",
        "OpenAIProviderProfile",
        "create_anthropic_profile",
        "create_gemini_profile",
        "create_openai_profile",
        "register_anthropic_tools",
        "register_gemini_tools",
        "build_openai_tool_registry",
        "register_openai_tools",
    }:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

    from .anthropic import (
        AnthropicProviderProfile,
        build_anthropic_tool_registry,
        create_anthropic_profile,
        register_anthropic_tools,
    )
    from .gemini import (
        GeminiProviderProfile,
        build_gemini_tool_registry,
        create_gemini_profile,
        register_gemini_tools,
    )
    from .openai import (
        OpenAIProviderProfile,
        build_openai_tool_registry,
        create_openai_profile,
        register_openai_tools,
    )

    exported = {
        "AnthropicProviderProfile": AnthropicProviderProfile,
        "build_anthropic_tool_registry": build_anthropic_tool_registry,
        "GeminiProviderProfile": GeminiProviderProfile,
        "build_gemini_tool_registry": build_gemini_tool_registry,
        "OpenAIProviderProfile": OpenAIProviderProfile,
        "create_anthropic_profile": create_anthropic_profile,
        "create_gemini_profile": create_gemini_profile,
        "create_openai_profile": create_openai_profile,
        "register_anthropic_tools": register_anthropic_tools,
        "register_gemini_tools": register_gemini_tools,
        "build_openai_tool_registry": build_openai_tool_registry,
        "register_openai_tools": register_openai_tools,
    }
    globals().update(exported)
    return exported[name]
