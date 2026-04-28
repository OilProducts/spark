from __future__ import annotations

import inspect
import logging
import os
from collections.abc import AsyncIterator, Mapping, Sequence
from dataclasses import replace
from typing import Any

from .adapters.base import ProviderAdapter
from .errors import ConfigurationError, SDKError
from .middleware import (
    CompleteMiddleware,
    Middleware,
    StreamMiddleware,
    complete_with_middleware,
    stream_with_middleware,
)
from .types import FinishReason, Request, Response, StreamEvent

logger = logging.getLogger(__name__)


class Client:
    def __init__(
        self,
        providers: Mapping[str, ProviderAdapter] | None = None,
        default_provider: str | None = None,
        *,
        middleware: Sequence[Middleware] | None = None,
        complete_middleware: Sequence[CompleteMiddleware] | None = None,
        stream_middleware: Sequence[StreamMiddleware] | None = None,
        **config: Any,
    ) -> None:
        self.providers = self._normalize_providers(providers)
        self.default_provider = self._normalize_provider_name(default_provider)
        common_middleware = list(middleware or ())
        self.middleware = common_middleware
        self.complete_middleware = common_middleware + list(complete_middleware or ())
        self.stream_middleware = common_middleware + list(stream_middleware or ())
        self.config = dict(config)
        self._initialize_registered_providers()

    @staticmethod
    def _normalize_provider_name(provider: str | None) -> str | None:
        if provider is None:
            return None
        if not isinstance(provider, str):
            raise TypeError("provider must be a string or None")
        return provider.casefold()

    def _normalize_providers(
        self,
        providers: Mapping[str, ProviderAdapter] | None,
    ) -> dict[str, ProviderAdapter]:
        normalized_providers: dict[str, ProviderAdapter] = {}
        for provider_name, adapter in dict(providers or {}).items():
            normalized_name = self._normalize_provider_name(provider_name)
            if normalized_name is None:
                raise TypeError("provider must be a string")
            normalized_providers[normalized_name] = adapter
        return normalized_providers

    def _initialize_registered_providers(self) -> None:
        for provider_name, adapter in self.providers.items():
            self._initialize_provider(provider_name, adapter)

    def _initialize_provider(self, provider_name: str, adapter: ProviderAdapter) -> None:
        initialize = getattr(adapter, "initialize", None)
        if not callable(initialize):
            return

        try:
            initialize()
        except SDKError:
            raise
        except Exception:
            logger.exception("Unexpected error initializing provider %s", provider_name)
            raise

    @classmethod
    def from_env(
        cls,
        providers: Mapping[str, ProviderAdapter] | None = None,
        default_provider: str | None = None,
        **config: Any,
    ) -> Client:
        logger.debug("Creating Client from environment")

        def _env_value(name: str) -> str | None:
            value = os.environ.get(name)
            if value is None or value == "":
                return None
            return value

        env_providers: dict[str, ProviderAdapter] = {}

        openai_api_key = _env_value("OPENAI_API_KEY")
        if openai_api_key is not None:
            from .adapters import OpenAIAdapter

            openai_config: dict[str, Any] = {"api_key": openai_api_key}
            if base_url := _env_value("OPENAI_BASE_URL"):
                openai_config["base_url"] = base_url
            if organization := _env_value("OPENAI_ORG_ID"):
                openai_config["organization"] = organization
            if project := _env_value("OPENAI_PROJECT_ID"):
                openai_config["project"] = project
            env_providers["openai"] = OpenAIAdapter(**openai_config)

        anthropic_api_key = _env_value("ANTHROPIC_API_KEY")
        if anthropic_api_key is not None:
            from .adapters import AnthropicAdapter

            anthropic_config: dict[str, Any] = {"api_key": anthropic_api_key}
            if base_url := _env_value("ANTHROPIC_BASE_URL"):
                anthropic_config["base_url"] = base_url
            env_providers["anthropic"] = AnthropicAdapter(**anthropic_config)

        gemini_api_key = _env_value("GEMINI_API_KEY")
        if gemini_api_key is None:
            gemini_api_key = _env_value("GOOGLE_API_KEY")
        if gemini_api_key is not None:
            from .adapters import GeminiAdapter

            gemini_config: dict[str, Any] = {"api_key": gemini_api_key}
            if base_url := _env_value("GEMINI_BASE_URL"):
                gemini_config["base_url"] = base_url
            env_providers["gemini"] = GeminiAdapter(**gemini_config)

        openrouter_api_key = _env_value("OPENROUTER_API_KEY")
        if openrouter_api_key is not None:
            from .adapters import OpenRouterAdapter

            openrouter_headers: dict[str, str] = {}
            if http_referer := _env_value("OPENROUTER_HTTP_REFERER"):
                openrouter_headers["HTTP-Referer"] = http_referer
            if title := _env_value("OPENROUTER_TITLE"):
                openrouter_headers["X-Title"] = title
            openrouter_config: dict[str, Any] = {"api_key": openrouter_api_key}
            if base_url := _env_value("OPENROUTER_BASE_URL"):
                openrouter_config["base_url"] = base_url
            if openrouter_headers:
                openrouter_config["default_headers"] = openrouter_headers
            env_providers["openrouter"] = OpenRouterAdapter(**openrouter_config)

        litellm_base_url = _env_value("LITELLM_BASE_URL")
        if litellm_base_url is not None:
            from .adapters import LiteLLMAdapter

            litellm_config: dict[str, Any] = {"base_url": litellm_base_url}
            if litellm_api_key := _env_value("LITELLM_API_KEY"):
                litellm_config["api_key"] = litellm_api_key
            env_providers["litellm"] = LiteLLMAdapter(**litellm_config)

        openai_compatible_base_url = _env_value("OPENAI_COMPATIBLE_BASE_URL")
        if openai_compatible_base_url is not None:
            from .adapters import OpenAICompatibleAdapter

            openai_compatible_config: dict[str, Any] = {
                "base_url": openai_compatible_base_url,
                "require_api_key": False,
            }
            if openai_compatible_api_key := _env_value("OPENAI_COMPATIBLE_API_KEY"):
                openai_compatible_config["api_key"] = openai_compatible_api_key
            env_providers["openai_compatible"] = OpenAICompatibleAdapter(**openai_compatible_config)

        merged_providers: dict[str, ProviderAdapter] = dict(env_providers)
        if providers:
            merged_providers.update(providers)

        resolved_default_provider = (
            default_provider
            if default_provider is not None
            else next(iter(env_providers or merged_providers), None)
        )
        return cls(
            providers=merged_providers,
            default_provider=resolved_default_provider,
            **config,
        )

    def _resolve_provider(self, provider: str | None) -> tuple[str, ProviderAdapter]:
        resolved_provider = self._normalize_provider_name(
            provider if provider is not None else self.default_provider
        )
        if resolved_provider is None:
            raise ConfigurationError(
                "No provider configured; set request.provider or Client.default_provider"
            )

        adapter = self.providers.get(resolved_provider)
        if adapter is None:
            raise ConfigurationError(f"Unknown provider {resolved_provider!r}")
        return resolved_provider, adapter

    def _prepare_request(self, request: Request) -> tuple[str, Request, ProviderAdapter]:
        resolved_provider, adapter = self._resolve_provider(request.provider)
        prepared_request = replace(request, provider=resolved_provider)
        return resolved_provider, prepared_request, adapter

    async def complete(self, request: Request | None = None) -> Response:
        if request is None or not isinstance(request, Request):
            raise TypeError("request must be a Request")

        _, prepared_request, adapter = self._prepare_request(request)
        return await complete_with_middleware(
            adapter.complete,
            prepared_request,
            self.complete_middleware,
        )

    def stream(self, request: Request | None = None) -> AsyncIterator[StreamEvent]:
        from .streaming import StreamEventIterator

        if request is None or not isinstance(request, Request):
            raise TypeError("request must be a Request")

        resolved_provider, prepared_request, adapter = self._prepare_request(request)
        source = stream_with_middleware(
            adapter.stream,
            prepared_request,
            self.stream_middleware,
        )
        return StreamEventIterator(
            source=source,
            response=Response(
                provider=resolved_provider,
                model=prepared_request.model,
                finish_reason=FinishReason(reason=FinishReason.STOP),
            ),
        )

    def supports_tool_choice(self, mode: str, provider: str | None = None) -> bool:
        resolved_provider, adapter = self._resolve_provider(provider)
        supports_tool_choice = getattr(adapter, "supports_tool_choice", None)
        if not callable(supports_tool_choice):
            return False

        try:
            return bool(supports_tool_choice(mode))
        except SDKError:
            raise
        except Exception:
            logger.exception(
                "Unexpected error checking tool choice support for provider %s",
                resolved_provider,
            )
            raise

    async def close(self) -> None:
        logger.debug("Client.close invoked")
        first_sdk_error: SDKError | None = None

        for provider_name, adapter in reversed(list(self.providers.items())):
            close = getattr(adapter, "close", None)
            if close is None or not callable(close):
                continue

            try:
                result = close()
                if inspect.isawaitable(result):
                    await result
            except SDKError as error:
                if first_sdk_error is None:
                    first_sdk_error = error
            except Exception:
                logger.exception("Unexpected error closing provider %s", provider_name)

        if first_sdk_error is not None:
            raise first_sdk_error
        return None


__all__ = ["Client"]
