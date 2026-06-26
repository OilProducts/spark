from __future__ import annotations

from typing import Final

import unified_llm


PROVIDER_ENV_KEYS: Final = (
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GEMINI_BASE_URL",
    "OPENROUTER_API_KEY",
    "OPENROUTER_BASE_URL",
    "OPENROUTER_HTTP_REFERER",
    "OPENROUTER_TITLE",
    "LITELLM_BASE_URL",
    "LITELLM_API_KEY",
    "OPENAI_COMPATIBLE_BASE_URL",
    "OPENAI_COMPATIBLE_API_KEY",
)


def test_python_unified_llm_remains_importable_compatibility_surface(monkeypatch) -> None:
    for key in PROVIDER_ENV_KEYS:
        monkeypatch.delenv(key, raising=False)

    client = unified_llm.Client.from_env()
    assert client.default_provider is None
    assert client.providers == {}

    request = unified_llm.Request(
        model="gpt-5.2",
        provider="openai",
        messages=[unified_llm.Message.user("hello")],
    )
    assert request.messages[0].text == "hello"

    sonnet = unified_llm.get_model_info("sonnet")
    assert sonnet is not None
    assert sonnet.provider == "anthropic"
    assert sonnet.id == "claude-sonnet-4-5"
    assert [model.id for model in unified_llm.list_models("gemini")] == [
        "gemini-3.1-pro-preview",
        "gemini-3-flash-preview",
    ]
