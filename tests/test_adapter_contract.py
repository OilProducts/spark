from __future__ import annotations

import importlib.metadata as metadata
import inspect
import json
import logging
import re
import subprocess
import sys
from collections.abc import AsyncIterable

import pytest

import unified_llm


def _requirement_name(requirement: str) -> str:
    return re.split(r"[ (<>=;]", requirement, maxsplit=1)[0]


def test_distribution_metadata_reflects_package_metadata() -> None:
    dist = metadata.distribution("spark")
    requirement_names = {_requirement_name(requirement) for requirement in dist.requires or []}

    assert dist.metadata["Name"] == "spark"
    assert {"httpx", "jsonschema"} <= requirement_names


def test_root_import_surface_exposes_async_first_public_surface() -> None:
    expected_names = {
        "AnthropicAdapter",
        "AudioData",
        "Client",
        "ConfigurationError",
        "ContentKind",
        "ContentPart",
        "DocumentData",
        "FinishReason",
        "GeminiAdapter",
        "GenerateResult",
        "ImageData",
        "Message",
        "ModelInfo",
        "OpenAIAdapter",
        "OpenAICompatibleAdapter",
        "ProviderAdapter",
        "ProviderError",
        "RateLimitInfo",
        "Request",
        "Response",
        "ResponseFormat",
        "Role",
        "SDKError",
        "StepResult",
        "StreamEvent",
        "StreamEventType",
        "StreamResult",
        "UnsupportedToolChoiceError",
        "Tool",
        "ToolCall",
        "ToolChoice",
        "ToolResult",
        "Usage",
        "Warning",
        "generate",
        "generate_object",
        "get_default_client",
        "get_latest_model",
        "get_model_info",
        "list_models",
        "set_default_client",
        "stream",
        "stream_object",
    }

    assert expected_names.issubset(set(unified_llm.__all__))
    for name in expected_names:
        assert hasattr(unified_llm, name)

    assert inspect.iscoroutinefunction(unified_llm.generate)
    assert inspect.iscoroutinefunction(unified_llm.generate_object)
    assert inspect.iscoroutinefunction(unified_llm.Client.complete)
    assert not inspect.iscoroutinefunction(unified_llm.stream)
    assert not inspect.iscoroutinefunction(unified_llm.Client.stream)


def test_catalog_helpers_are_import_safe_and_advisory() -> None:
    assert unified_llm.get_model_info("gpt-5.2").provider == "openai"
    assert unified_llm.get_model_info("sonnet").id == "claude-sonnet-4-5"
    assert [model.id for model in unified_llm.list_models("gemini")] == [
        "gemini-3.1-pro-preview",
        "gemini-3-flash-preview",
    ]
    assert unified_llm.get_latest_model("openai").id == "gpt-5.2"
    assert unified_llm.get_model_info("missing-model") is None
    assert unified_llm.get_latest_model("missing-provider") is None


@pytest.mark.asyncio
async def test_client_complete_requires_a_request() -> None:
    with pytest.raises(TypeError, match="request must be a Request"):
        await unified_llm.Client().complete()


def test_client_stream_requires_a_request() -> None:
    with pytest.raises(TypeError, match="request must be a Request"):
        unified_llm.Client().stream()


def test_stream_requires_prompt_or_messages() -> None:
    with pytest.raises(ValueError, match="either prompt or messages"):
        unified_llm.stream()


@pytest.mark.asyncio
async def test_generate_object_requires_a_schema() -> None:
    with pytest.raises(TypeError, match="schema must be provided"):
        await unified_llm.generate_object()


def test_stream_object_requires_a_schema() -> None:
    with pytest.raises(TypeError, match="schema must be provided"):
        unified_llm.stream_object()


@pytest.mark.asyncio
async def test_openai_adapter_surfaces_configuration_and_stream_errors_through_real_interfaces(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)

    adapter = unified_llm.OpenAIAdapter()
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    with pytest.raises(
        unified_llm.ConfigurationError,
        match="OpenAI API key is required",
    ):
        await adapter.complete(request)

    stream = adapter.stream(request)

    assert isinstance(stream, AsyncIterable)

    with pytest.raises(
        unified_llm.ConfigurationError,
        match="OpenAI API key is required",
    ):
        await stream.__anext__()


def test_default_client_round_trip() -> None:
    previous = unified_llm.get_default_client()
    client = unified_llm.Client.from_env()

    try:
        unified_llm.set_default_client(client)
        assert unified_llm.get_default_client() is client
    finally:
        unified_llm.set_default_client(previous)


@pytest.mark.asyncio
async def test_provider_adapter_protocol_accepts_fake_async_adapter() -> None:
    class _FakeStream:
        def __init__(self) -> None:
            self._yielded = False

        def __aiter__(self) -> _FakeStream:
            return self

        async def __anext__(self) -> unified_llm.StreamEvent:
            if self._yielded:
                raise StopAsyncIteration
            self._yielded = True
            return unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.PROVIDER_EVENT,
                raw={"kind": "fake"},
            )

    class _FakeAdapter:
        name = "fake"

        async def complete(self, request: object) -> unified_llm.Response:
            return unified_llm.Response()

        def stream(self, request: object) -> _FakeStream:
            return _FakeStream()

    adapter = _FakeAdapter()
    events = [event async for event in adapter.stream(object())]

    assert isinstance(adapter, unified_llm.ProviderAdapter)
    assert not isinstance(adapter, unified_llm.SupportsInitialize)
    assert not isinstance(adapter, unified_llm.SupportsClose)
    assert not isinstance(adapter, unified_llm.SupportsToolChoice)
    assert isinstance(_FakeAdapter().stream(object()), AsyncIterable)
    assert events == [
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.PROVIDER_EVENT,
            raw={"kind": "fake"},
        )
    ]
    assert not hasattr(unified_llm.ProviderAdapter, "send_tool_outputs")
    assert not hasattr(unified_llm.ProviderAdapter, "initialize")
    assert not hasattr(unified_llm.ProviderAdapter, "supports_tool_choice")


def test_client_does_not_accumulate_per_request_state() -> None:
    client = unified_llm.Client(
        providers={"fake": unified_llm.OpenAIAdapter()},
        default_provider="fake",
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )
    snapshot = dict(client.__dict__)

    stream = client.stream(request)
    assert dict(client.__dict__) == snapshot
    assert isinstance(stream, AsyncIterable)


def test_adapter_protocols_are_publicly_exported_from_adapter_namespace() -> None:
    from unified_llm.adapters import (
        ProviderAdapter as AdapterProviderAdapter,
    )
    from unified_llm.adapters import (
        SupportsClose as AdapterSupportsClose,
    )
    from unified_llm.adapters import (
        SupportsInitialize as AdapterSupportsInitialize,
    )
    from unified_llm.adapters import (
        SupportsToolChoice as AdapterSupportsToolChoice,
    )

    assert AdapterProviderAdapter is unified_llm.ProviderAdapter
    assert AdapterSupportsInitialize is unified_llm.SupportsInitialize
    assert AdapterSupportsClose is unified_llm.SupportsClose
    assert AdapterSupportsToolChoice is unified_llm.SupportsToolChoice


def test_support_protocols_are_runtime_checkable() -> None:
    class _SupportedAdapter:
        def initialize(self) -> None:
            return None

        def close(self) -> None:
            return None

        def supports_tool_choice(self, mode: str) -> bool:
            return mode == "required"

    adapter = _SupportedAdapter()

    assert isinstance(adapter, unified_llm.SupportsInitialize)
    assert isinstance(adapter, unified_llm.SupportsClose)
    assert isinstance(adapter, unified_llm.SupportsToolChoice)


def test_importing_the_root_package_does_not_load_later_layers() -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            (
                "import json, sys, unified_llm; "
                "print(json.dumps(sorted(m for m in sys.modules if m.startswith('unified_llm'))))"
            ),
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    loaded_modules = json.loads(completed.stdout)
    assert completed.stderr == ""
    assert loaded_modules == ["unified_llm"]
    assert "unified_llm.streaming" not in loaded_modules


def test_layer1_module_imports_do_not_pull_high_level_apis() -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            (
                "import json, sys, unified_llm.types; "
                "print(json.dumps(sorted(m for m in sys.modules if m.startswith('unified_llm'))))"
            ),
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    loaded_modules = set(json.loads(completed.stdout))
    assert completed.stderr == ""
    assert "unified_llm" in loaded_modules
    assert "unified_llm.types" in loaded_modules
    assert "unified_llm.errors" in loaded_modules
    assert "unified_llm.client" not in loaded_modules
    assert "unified_llm.defaults" not in loaded_modules
    assert "unified_llm.generation" not in loaded_modules
    assert "unified_llm.models" not in loaded_modules
    assert "unified_llm.structured" not in loaded_modules
    assert "unified_llm.streaming" not in loaded_modules
    assert "unified_llm.tools" not in loaded_modules


def test_adapter_protocol_imports_do_not_pull_high_level_apis() -> None:
    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            (
                "import json, sys, unified_llm.adapters.base; "
                "print(json.dumps(sorted(m for m in sys.modules if m.startswith('unified_llm'))))"
            ),
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    loaded_modules = set(json.loads(completed.stdout))
    assert completed.stderr == ""
    assert "unified_llm" in loaded_modules
    assert "unified_llm.adapters" in loaded_modules
    assert "unified_llm.adapters.base" in loaded_modules
    assert "unified_llm.client" not in loaded_modules
    assert "unified_llm.defaults" not in loaded_modules
    assert "unified_llm.generation" not in loaded_modules
    assert "unified_llm.models" not in loaded_modules
    assert "unified_llm.structured" not in loaded_modules
    assert "unified_llm.tools" not in loaded_modules


def test_catalog_and_configuration_apis_log_through_module_loggers_without_printing(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    with caplog.at_level(logging.DEBUG):
        assert unified_llm.get_model_info("gpt-5.2").id == "gpt-5.2"
        assert [model.id for model in unified_llm.list_models("openai")] == [
            "gpt-5.2",
            "gpt-5.2-mini",
            "gpt-5.2-codex",
        ]
        unified_llm.Client.from_env()

    captured = capsys.readouterr()
    logger_names = {record.name for record in caplog.records}

    assert captured.out == ""
    assert captured.err == ""
    assert {"unified_llm.models", "unified_llm.client"} <= logger_names
