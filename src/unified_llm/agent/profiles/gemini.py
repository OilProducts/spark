from __future__ import annotations

import re
from collections.abc import Mapping
from dataclasses import dataclass
from functools import partial
from html import unescape
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, quote_plus, unquote, urljoin, urlparse
from urllib.request import Request as URLRequest
from urllib.request import urlopen

from ...models import get_model_info
from ...tools import ToolResult
from ..builtin_tools import DEFAULT_READ_FILE_LIMIT, builtin_tool_definitions
from ..builtin_tools import edit_file as builtin_edit_file
from ..builtin_tools import glob as builtin_glob
from ..builtin_tools import grep as builtin_grep
from ..builtin_tools import list_dir as builtin_list_dir
from ..builtin_tools import read_file as builtin_read_file
from ..builtin_tools import read_many_files as builtin_read_many_files
from ..builtin_tools import shell as builtin_shell
from ..builtin_tools import write_file as builtin_write_file
from ..subagents import register_subagent_tools
from ..tools import RegisteredTool, ToolDefinition, ToolRegistry
from .base import ProviderProfile

DEFAULT_GEMINI_SHELL_TIMEOUT_MS = 10_000
DEFAULT_GEMINI_WEB_SEARCH_RESULTS = 5
DEFAULT_GEMINI_WEB_FETCH_BYTES = 20_000
DEFAULT_GEMINI_WEB_FETCH_TIMEOUT_MS = 10_000


def create_gemini_profile(*args: Any, **kwargs: Any) -> GeminiProviderProfile:
    return GeminiProviderProfile(*args, **kwargs)


def _tool_result(
    content: str | dict[str, Any] | list[Any],
    *,
    is_error: bool,
    image_data: bytes | None = None,
    image_media_type: str | None = None,
) -> ToolResult:
    return ToolResult(
        content=content,
        is_error=is_error,
        image_data=image_data,
        image_media_type=image_media_type,
    )


def _error(message: str) -> ToolResult:
    return _tool_result(message, is_error=True)


def _object_tool_definition(
    name: str,
    description: str,
    *,
    properties: Mapping[str, Any],
    required: list[str],
) -> ToolDefinition:
    return ToolDefinition(
        name=name,
        description=description,
        parameters={
            "type": "object",
            "properties": dict(properties),
            "required": list(required),
            "additionalProperties": False,
        },
    )


def _builtin_definition_map() -> dict[str, ToolDefinition]:
    definitions = builtin_tool_definitions()
    return {definition.name: definition for definition in definitions}


def _gemini_read_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "read_file",
        "Read a file and return line-numbered text or image data.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "offset": {"type": "integer", "minimum": 1, "default": 1},
            "limit": {
                "type": "integer",
                "minimum": 0,
                "default": DEFAULT_READ_FILE_LIMIT,
            },
        },
        required=["file_path"],
    )


def _gemini_write_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "write_file",
        "Write a file and report how many bytes were written.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "content": {"type": "string"},
        },
        required=["file_path", "content"],
    )


def _gemini_edit_file_definition() -> ToolDefinition:
    return _object_tool_definition(
        "edit_file",
        "Edit a file with search-and-replace semantics.",
        properties={
            "file_path": {"type": "string", "minLength": 1},
            "instruction": {"type": "string", "minLength": 1},
            "old_string": {"type": "string", "minLength": 1},
            "new_string": {"type": "string"},
            "allow_multiple": {"type": "boolean", "default": False},
        },
        required=["file_path", "instruction", "old_string", "new_string"],
    )


def _gemini_shell_definition() -> ToolDefinition:
    return _object_tool_definition(
        "shell",
        "Run a shell command and return stdout, stderr, and exit metadata.",
        properties={
            "command": {"type": "string", "minLength": 1},
            "description": {"type": "string"},
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_GEMINI_SHELL_TIMEOUT_MS,
            },
        },
        required=["command"],
    )


def _web_search_definition() -> ToolDefinition:
    return _object_tool_definition(
        "web_search",
        "Search the web for up-to-date information.",
        properties={
            "query": {"type": "string", "minLength": 1},
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_GEMINI_WEB_SEARCH_RESULTS,
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_GEMINI_WEB_FETCH_TIMEOUT_MS,
            },
        },
        required=["query"],
    )


def _web_fetch_definition() -> ToolDefinition:
    return _object_tool_definition(
        "web_fetch",
        "Fetch and extract content from a URL.",
        properties={
            "url": {"type": "string", "minLength": 1},
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_GEMINI_WEB_FETCH_BYTES,
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_GEMINI_WEB_FETCH_TIMEOUT_MS,
            },
        },
        required=["url"],
    )


def _sanitize_text(text: str) -> str:
    text = re.sub(r"(?is)<(script|style).*?>.*?</\1>", " ", text)
    text = re.sub(r"(?s)<[^>]+>", " ", text)
    text = unescape(text)
    text = re.sub(r"\s+", " ", text)
    return text.strip()


def _decode_response_body(
    data: bytes,
    content_type: str | None,
    charset: str | None,
    *,
    strip_html: bool = True,
) -> str:
    decoded = data.decode(charset or "utf-8", errors="replace")
    if strip_html and content_type is not None and "html" in content_type.casefold():
        return _sanitize_text(decoded)
    return decoded.strip()


def _normalize_search_href(href: str) -> str:
    absolute_href = urljoin("https://duckduckgo.com", unescape(href))
    parsed = urlparse(absolute_href)
    if parsed.path == "/l/" and parsed.query:
        query = parse_qs(parsed.query)
        for key in ("uddg", "u"):
            value = query.get(key)
            if value:
                return unquote(value[0])
    return absolute_href


def _search_results_from_html(html_text: str, max_results: int) -> list[dict[str, str]]:
    pattern = re.compile(
        r'<a[^>]*class="[^"]*result__[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>',
        re.IGNORECASE | re.DOTALL,
    )
    results: list[dict[str, str]] = []
    seen: set[str] = set()
    for href, raw_title in pattern.findall(html_text):
        url = _normalize_search_href(href)
        if url in seen:
            continue
        seen.add(url)
        title = _sanitize_text(raw_title)
        if not title:
            continue
        results.append({"title": title, "url": url})
        if len(results) >= max_results:
            break
    return results


def _web_search(
    arguments: Mapping[str, Any],
    execution_environment: Any,
    provider_profile: Any | None = None,
    session_config: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    query = arguments.get("query")
    if not isinstance(query, str) or not query.strip():
        return _error("Missing required argument: query")

    max_results = arguments.get("max_results", DEFAULT_GEMINI_WEB_SEARCH_RESULTS)
    if (
        not isinstance(max_results, int)
        or isinstance(max_results, bool)
        or max_results < 1
    ):
        return _error("max_results must be at least 1")

    timeout_ms = arguments.get("timeout_ms", DEFAULT_GEMINI_WEB_FETCH_TIMEOUT_MS)
    if (
        not isinstance(timeout_ms, int)
        or isinstance(timeout_ms, bool)
        or timeout_ms < 1
    ):
        return _error("timeout_ms must be at least 1")

    search_url = f"https://html.duckduckgo.com/html/?q={quote_plus(query)}"
    request = URLRequest(
        search_url,
        headers={"User-Agent": "unified_llm-gemini-profile/1.0"},
    )
    try:
        with urlopen(request, timeout=timeout_ms / 1000) as response:
            body = response.read(DEFAULT_GEMINI_WEB_FETCH_BYTES)
            content_type = response.headers.get_content_type()
            charset = response.headers.get_content_charset()
    except (HTTPError, URLError, TimeoutError, OSError) as exc:
        return _error(f"web_search failed: {exc}")

    html_text = _decode_response_body(
        body,
        content_type,
        charset,
        strip_html=False,
    )
    results = _search_results_from_html(html_text, max_results)
    return _tool_result(
        {
            "query": query,
            "source": "duckduckgo",
            "results": results,
        },
        is_error=False,
    )


def _web_fetch(
    arguments: Mapping[str, Any],
    execution_environment: Any,
    provider_profile: Any | None = None,
    session_config: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    url = arguments.get("url")
    if not isinstance(url, str) or not url.strip():
        return _error("Missing required argument: url")

    max_bytes = arguments.get("max_bytes", DEFAULT_GEMINI_WEB_FETCH_BYTES)
    if not isinstance(max_bytes, int) or isinstance(max_bytes, bool) or max_bytes < 1:
        return _error("max_bytes must be at least 1")

    timeout_ms = arguments.get("timeout_ms", DEFAULT_GEMINI_WEB_FETCH_TIMEOUT_MS)
    if not isinstance(timeout_ms, int) or isinstance(timeout_ms, bool) or timeout_ms < 1:
        return _error("timeout_ms must be at least 1")

    request = URLRequest(url, headers={"User-Agent": "unified_llm-gemini-profile/1.0"})
    try:
        with urlopen(request, timeout=timeout_ms / 1000) as response:
            body = response.read(max_bytes + 1)
            final_url = response.geturl()
            status_code = getattr(response, "status", response.getcode())
            content_type = response.headers.get_content_type()
            charset = response.headers.get_content_charset()
    except (HTTPError, URLError, TimeoutError, OSError) as exc:
        return _error(f"web_fetch failed: {exc}")

    truncated = len(body) > max_bytes
    body = body[:max_bytes]
    content = _decode_response_body(body, content_type, charset)
    return _tool_result(
        {
            "url": final_url,
            "status_code": status_code,
            "content_type": content_type,
            "truncated": truncated,
            "content": content,
        },
        is_error=False,
    )


def edit_file(
    arguments: Mapping[str, Any],
    execution_environment: Any,
    provider_profile: Any | None = None,
) -> ToolResult:
    if not isinstance(arguments, Mapping):
        return _error("arguments must be a mapping")

    normalized_arguments = dict(arguments)
    allow_multiple = normalized_arguments.pop("allow_multiple", False)
    normalized_arguments.pop("instruction", None)
    normalized_arguments["replace_all"] = bool(allow_multiple)
    return builtin_edit_file(
        normalized_arguments,
        execution_environment,
        provider_profile=provider_profile,
    )


def register_gemini_tools(
    registry: ToolRegistry | None = None,
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    target_registry = registry if registry is not None else ToolRegistry()
    builtin_definition_map = _builtin_definition_map()
    tool_definitions = [
        _gemini_read_file_definition(),
        builtin_definition_map["read_many_files"],
        _gemini_write_file_definition(),
        _gemini_edit_file_definition(),
        _gemini_shell_definition(),
        builtin_definition_map["grep"],
        builtin_definition_map["glob"],
        builtin_definition_map["list_dir"],
    ]
    if bool(getattr(provider_profile, "enable_web_search", False)):
        tool_definitions.append(_web_search_definition())
    if bool(getattr(provider_profile, "enable_web_fetch", False)):
        tool_definitions.append(_web_fetch_definition())

    executor_map = {
        "read_file": partial(builtin_read_file, provider_profile=provider_profile),
        "read_many_files": partial(
            builtin_read_many_files,
            provider_profile=provider_profile,
        ),
        "write_file": partial(builtin_write_file, provider_profile=provider_profile),
        "edit_file": edit_file,
        "shell": partial(builtin_shell, provider_profile=provider_profile),
        "grep": partial(builtin_grep, provider_profile=provider_profile),
        "glob": partial(builtin_glob, provider_profile=provider_profile),
        "list_dir": partial(builtin_list_dir, provider_profile=provider_profile),
        "web_search": _web_search,
        "web_fetch": _web_fetch,
    }
    for definition in tool_definitions:
        target_registry.register(
            RegisteredTool(
                definition=definition,
                executor=executor_map[definition.name],
                metadata={
                    "kind": "web" if definition.name in {"web_search", "web_fetch"} else "builtin"
                },
            )
        )
    register_subagent_tools(target_registry, provider_profile=provider_profile)
    return target_registry


def build_gemini_tool_registry(
    *,
    provider_profile: Any | None = None,
) -> ToolRegistry:
    return register_gemini_tools(provider_profile=provider_profile)


@dataclass(slots=True)
class GeminiProviderProfile(ProviderProfile):
    enable_web_search: bool = False
    enable_web_fetch: bool = False
    shell_timeout_ms: int = DEFAULT_GEMINI_SHELL_TIMEOUT_MS

    def __post_init__(self) -> None:
        custom_tool_registry = self.tool_registry
        ProviderProfile.__post_init__(self)

        if not isinstance(self.enable_web_search, bool):
            raise TypeError("enable_web_search must be a boolean")
        if not isinstance(self.enable_web_fetch, bool):
            raise TypeError("enable_web_fetch must be a boolean")
        if (
            not isinstance(self.shell_timeout_ms, int)
            or isinstance(self.shell_timeout_ms, bool)
            or self.shell_timeout_ms < 1
        ):
            raise ValueError("shell_timeout_ms must be at least 1")

        if not self.id:
            self.id = "gemini"

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
            self.display_name = "Gemini"

        self.tool_registry = build_gemini_tool_registry(provider_profile=self)
        if custom_tool_registry:
            if not isinstance(custom_tool_registry, ToolRegistry):
                custom_tool_registry = ToolRegistry(custom_tool_registry)
            for name, tool in custom_tool_registry.items():
                self.tool_registry.register(tool, name=name)


__all__ = [
    "DEFAULT_GEMINI_SHELL_TIMEOUT_MS",
    "GeminiProviderProfile",
    "build_gemini_tool_registry",
    "create_gemini_profile",
    "register_gemini_tools",
]
