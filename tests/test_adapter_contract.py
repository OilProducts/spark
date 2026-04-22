from __future__ import annotations

import importlib.metadata as metadata
import inspect
import json
import logging
import re
import subprocess
import sys
from collections.abc import AsyncIterable
from pathlib import Path

import pytest

import unified_llm


def _requirement_name(requirement: str) -> str:
    return re.split(r"[ (<>=;]", requirement, maxsplit=1)[0]


def test_distribution_metadata_reflects_the_uv_scaffold_contract() -> None:
    dist = metadata.distribution("unified_llm")
    package_path = Path(unified_llm.__file__).resolve()
    requirement_names = {_requirement_name(requirement) for requirement in dist.requires or []}

    assert dist.metadata["Name"] == "unified_llm"
    assert package_path.parent.name == "unified_llm"
    assert package_path.parent.parent.name == "src"
    assert {"httpx", "jsonschema"} <= requirement_names


def test_root_import_surface_exposes_async_first_placeholders() -> None:
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


def test_catalog_helpers_are_import_safe_placeholders() -> None:
    assert unified_llm.get_model_info("gpt-5.2") is None
    assert unified_llm.get_latest_model("openai") is None
    assert unified_llm.list_models() == []


def test_stream_placeholders_are_async_iterables() -> None:
    client = unified_llm.Client()
    stream = unified_llm.stream()
    client_stream = client.stream()
    structured_stream = unified_llm.stream_object()
    adapter_stream = unified_llm.OpenAIAdapter().stream(object())

    assert isinstance(stream, AsyncIterable)
    assert isinstance(client_stream, AsyncIterable)
    assert isinstance(structured_stream, AsyncIterable)
    assert isinstance(adapter_stream, AsyncIterable)
    assert isinstance(stream, unified_llm.StreamResult)
    assert isinstance(structured_stream, unified_llm.StreamResult)
    assert not isinstance(client_stream, unified_llm.StreamResult)
    assert not isinstance(adapter_stream, unified_llm.StreamResult)


@pytest.mark.asyncio
async def test_awaitable_placeholders_raise_when_used() -> None:
    with pytest.raises(NotImplementedError):
        await unified_llm.generate()

    with pytest.raises(NotImplementedError):
        await unified_llm.generate_object()

    with pytest.raises(NotImplementedError):
        await unified_llm.Client().complete()


def test_default_client_round_trip() -> None:
    previous = unified_llm.get_default_client()
    client = unified_llm.Client.from_env()

    try:
        unified_llm.set_default_client(client)
        assert unified_llm.get_default_client() is client
    finally:
        unified_llm.set_default_client(previous)


def test_provider_adapter_protocol_accepts_fake_async_adapter() -> None:
    class _FakeStream:
        def __aiter__(self) -> _FakeStream:
            return self

        async def __anext__(self) -> object:
            raise StopAsyncIteration

    class _FakeAdapter:
        name = "fake"

        async def complete(self, request: object) -> object:
            return unified_llm.Response()

        def stream(self, request: object) -> _FakeStream:
            return _FakeStream()

    adapter = _FakeAdapter()

    assert isinstance(adapter, unified_llm.ProviderAdapter)
    assert isinstance(adapter.stream(object()), AsyncIterable)
    assert not hasattr(unified_llm.ProviderAdapter, "send_tool_outputs")


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


def test_placeholder_apis_log_through_module_loggers_without_printing(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    with caplog.at_level(logging.DEBUG):
        assert unified_llm.get_model_info("gpt-5.2") is None
        assert unified_llm.stream()
        unified_llm.Client.from_env()

    captured = capsys.readouterr()
    logger_names = {record.name for record in caplog.records}

    assert captured.out == ""
    assert captured.err == ""
    assert {"unified_llm.models", "unified_llm.generation", "unified_llm.client"} <= logger_names
