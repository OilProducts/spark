"""Public re-export surface for the unified_llm package."""

from __future__ import annotations

import logging
from importlib import import_module
from typing import Any

logger = logging.getLogger(__name__)

__all__ = [
    "AnthropicAdapter",
    "AudioData",
    "AbortError",
    "AccessDeniedError",
    "AuthenticationError",
    "Client",
    "ConfigurationError",
    "ContentFilterError",
    "ContentKind",
    "ContentPart",
    "ContextLengthError",
    "DocumentData",
    "FinishReason",
    "GeminiAdapter",
    "GenerateResult",
    "ImageData",
    "InvalidRequestError",
    "InvalidToolCallError",
    "Message",
    "ModelInfo",
    "NetworkError",
    "NoObjectGeneratedError",
    "NotFoundError",
    "OpenAIAdapter",
    "OpenAICompatibleAdapter",
    "ProviderAdapter",
    "ProviderError",
    "QuotaExceededError",
    "RateLimitError",
    "RateLimitInfo",
    "Request",
    "RequestTimeoutError",
    "Response",
    "ResponseFormat",
    "Role",
    "SDKError",
    "ServerError",
    "StepResult",
    "StreamEvent",
    "StreamEventType",
    "StreamResult",
    "StreamError",
    "UnsupportedToolChoiceError",
    "SupportsClose",
    "SupportsInitialize",
    "SupportsToolChoice",
    "ThinkingData",
    "Tool",
    "ToolCall",
    "ToolCallData",
    "ToolChoice",
    "ToolResult",
    "ToolResultData",
    "Usage",
    "Warning",
    "classify_provider_error_message",
    "generate",
    "generate_object",
    "get_default_client",
    "get_latest_model",
    "get_model_info",
    "error_from_grpc_code",
    "error_from_status_code",
    "list_models",
    "set_default_client",
    "stream",
    "stream_object",
]

_MODULE_EXPORTS: dict[str, tuple[str, ...]] = {
    "adapters": (
        "AnthropicAdapter",
        "GeminiAdapter",
        "OpenAIAdapter",
        "OpenAICompatibleAdapter",
    ),
    "adapters.base": (
        "ProviderAdapter",
        "SupportsClose",
        "SupportsInitialize",
        "SupportsToolChoice",
    ),
    "client": ("Client",),
    "defaults": ("get_default_client", "set_default_client"),
    "errors": (
        "AbortError",
        "AccessDeniedError",
        "AuthenticationError",
        "ConfigurationError",
        "ContentFilterError",
        "ContextLengthError",
        "classify_provider_error_message",
        "InvalidRequestError",
        "InvalidToolCallError",
        "NetworkError",
        "NoObjectGeneratedError",
        "NotFoundError",
        "error_from_grpc_code",
        "error_from_status_code",
        "ProviderError",
        "QuotaExceededError",
        "RateLimitError",
        "RequestTimeoutError",
        "SDKError",
        "ServerError",
        "StreamError",
        "UnsupportedToolChoiceError",
    ),
    "generation": ("GenerateResult", "StepResult", "StreamResult", "generate", "stream"),
    "models": ("ModelInfo", "get_latest_model", "get_model_info", "list_models"),
    "structured": ("generate_object", "stream_object"),
    "tools": ("Tool", "ToolCall", "ToolChoice", "ToolResult"),
    "types": (
        "AudioData",
        "ContentKind",
        "ContentPart",
        "DocumentData",
        "FinishReason",
        "ImageData",
        "Message",
        "RateLimitInfo",
        "Request",
        "Response",
        "ResponseFormat",
        "Role",
        "StreamEvent",
        "StreamEventType",
        "ThinkingData",
        "ToolCallData",
        "ToolResultData",
        "Usage",
        "Warning",
    ),
}

_NAME_TO_MODULE = {
    export_name: module_name
    for module_name, export_names in _MODULE_EXPORTS.items()
    for export_name in export_names
}


def __getattr__(name: str) -> Any:
    module_name = _NAME_TO_MODULE.get(name)
    if module_name is None:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

    module = import_module(f".{module_name}", __name__)
    for export_name in _MODULE_EXPORTS[module_name]:
        globals()[export_name] = getattr(module, export_name)
    return globals()[name]


def __dir__() -> list[str]:
    return sorted({*globals(), *__all__})
