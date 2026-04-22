from __future__ import annotations

import inspect
import logging
from collections.abc import AsyncIterator, Mapping
from typing import Any

import httpx

from ..errors import ConfigurationError
from ..provider_utils.errors import provider_error_from_response
from ..provider_utils.http import provider_options_for
from ..provider_utils.normalization import normalize_raw_payload
from ..provider_utils.openai_compatible import (
    _provider_error_from_httpx_error,
    _StreamState,
    build_openai_compatible_chat_completions_request,
    build_openai_compatible_chat_completions_url,
    normalize_openai_compatible_base_url,
    normalize_openai_compatible_response,
)
from ..types import Response, StreamEvent, StreamEventType

logger = logging.getLogger(__name__)


class OpenAICompatibleAdapter:
    name = "openai_compatible"

    def __init__(
        self,
        api_key: str | None = None,
        base_url: str | None = None,
        timeout: float | httpx.Timeout | None = None,
        default_headers: Mapping[str, Any] | None = None,
        client: httpx.AsyncClient | Any | None = None,
        transport: httpx.AsyncBaseTransport | None = None,
        *,
        owns_client: bool = False,
    ) -> None:
        if client is not None and transport is not None:
            raise ValueError("client and transport are mutually exclusive")

        self.api_key = api_key
        self.base_url = normalize_openai_compatible_base_url(base_url)
        self.timeout = timeout
        self.default_headers = dict(default_headers or {})
        self.config = {
            "api_key": self.api_key,
            "base_url": self.base_url,
        }
        self._chat_completions_url = build_openai_compatible_chat_completions_url(self.base_url)
        self._client = client
        self._owns_client = owns_client or client is None
        self._client_closed = False

        if self._client is None:
            client_kwargs: dict[str, Any] = {}
            if transport is not None:
                client_kwargs["transport"] = transport
            if self.timeout is not None:
                client_kwargs["timeout"] = self.timeout
            self._client = httpx.AsyncClient(**client_kwargs)

    def _request_kwargs(
        self,
        request: Any,
        *,
        stream: bool = False,
    ) -> dict[str, Any]:
        provider_options = provider_options_for(request, self.name)
        body, header_overrides = build_openai_compatible_chat_completions_request(
            request,
            provider_options=provider_options,
            stream=stream,
        )

        headers = httpx.Headers(self.default_headers)
        if header_overrides is not None:
            headers.update(header_overrides)

        if self.api_key is None:
            raise ConfigurationError("OpenAI-compatible API key is required")
        headers["Authorization"] = f"Bearer {self.api_key}"

        kwargs: dict[str, Any] = {
            "headers": headers,
            "json": body,
        }
        if self.timeout is not None:
            kwargs["timeout"] = self.timeout
        return kwargs

    async def complete(self, request: Any) -> Response:
        if not hasattr(request, "messages"):
            raise TypeError("request must be a Request")

        client = self._client
        if client is None:
            raise ConfigurationError("OpenAI-compatible HTTP client is not available")

        try:
            response = await client.post(
                self._chat_completions_url, **self._request_kwargs(request)
            )
        except httpx.HTTPError as exc:
            raise _provider_error_from_httpx_error(exc, provider=self.name) from exc

        if response.status_code >= 400:
            await response.aread()
            raise provider_error_from_response(
                response,
                provider=self.name,
                raw=normalize_raw_payload(response.text),
            )

        payload = normalize_raw_payload(response.text)
        return normalize_openai_compatible_response(
            payload,
            provider=self.name,
            headers=response.headers,
            raw=payload,
        )

    def stream(self, request: Any) -> AsyncIterator[StreamEvent]:
        async def _stream() -> AsyncIterator[StreamEvent]:
            if not hasattr(request, "messages"):
                raise TypeError("request must be a Request")

            client = self._client
            if client is None:
                raise ConfigurationError("OpenAI-compatible HTTP client is not available")

            try:
                async with client.stream(
                    "POST",
                    self._chat_completions_url,
                    **self._request_kwargs(request, stream=True),
                ) as response:
                    if response.status_code >= 400:
                        await response.aread()
                        raise provider_error_from_response(
                            response,
                            provider=self.name,
                            raw=normalize_raw_payload(response.text),
                        )

                    content_type = response.headers.get("content-type", "")
                    if "text/event-stream" not in content_type.casefold():
                        payload = normalize_raw_payload(await response.aread())
                        normalized = normalize_openai_compatible_response(
                            payload,
                            provider=self.name,
                            headers=response.headers,
                            raw=payload,
                        )
                        yield StreamEvent(
                            type=StreamEventType.STREAM_START,
                            response=normalized,
                            raw=payload,
                        )
                        yield StreamEvent(
                            type=StreamEventType.FINISH,
                            finish_reason=normalized.finish_reason,
                            usage=normalized.usage,
                            response=normalized,
                            raw=payload,
                        )
                        return

                    state = _StreamState(provider=self.name, headers=response.headers)
                    async for event in normalize_stream_events(response, state):
                        yield event
                        if event.type in (StreamEventType.FINISH, StreamEventType.ERROR):
                            return
                    if state.started or state.last_payload is not None:
                        for event in state.finalize():
                            yield event
            except httpx.HTTPError as exc:
                raise _provider_error_from_httpx_error(exc, provider=self.name) from exc

        return _stream()

    def supports_tool_choice(self, mode: str) -> bool:
        return mode.casefold() in {"auto", "none", "required", "named"}

    async def close(self) -> None:
        if self._client_closed or not self._owns_client:
            return None

        client = self._client
        if client is None:
            self._client_closed = True
            return None

        close = getattr(client, "aclose", None)
        if close is None or not callable(close):
            self._client_closed = True
            return None

        try:
            result = close()
            if inspect.isawaitable(result):
                await result
            self._client_closed = True
        except Exception:
            logger.exception("Unexpected error closing OpenAI-compatible HTTP client")


async def normalize_stream_events(
    response: httpx.Response,
    state: _StreamState,
) -> AsyncIterator[StreamEvent]:
    from ..provider_utils.sse import aiter_sse_events

    async for event in aiter_sse_events(response.aiter_lines()):
        payload = normalize_raw_payload(event.data)
        for translated_event in state.translate(payload):
            yield translated_event


__all__ = ["OpenAICompatibleAdapter"]
