from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any

from ..environment import ExecutionEnvironment
from ..prompts import build_system_prompt as build_layered_system_prompt
from ..tools import RegisteredTool, ToolDefinition, ToolRegistry


def _copy_mapping(mapping: Mapping[str, Any] | None) -> dict[str, Any]:
    if mapping is None:
        return {}
    return dict(mapping)


def _normalize_capability_name(capability: str) -> str:
    return capability.casefold().removeprefix("supports_")


@dataclass(slots=True)
class ProviderProfile:
    id: str = ""
    model: str = ""
    subagent_model_overrides: tuple[str, ...] | list[str] = field(default_factory=tuple)
    tool_registry: ToolRegistry | Mapping[str, RegisteredTool | ToolDefinition] = field(
        default_factory=ToolRegistry
    )
    capabilities: dict[str, bool] = field(default_factory=dict)
    provider_options_map: dict[str, Any] = field(default_factory=dict)
    context_window_size: int | None = None
    display_name: str | None = None
    knowledge_cutoff: str | None = None
    knowledge_cutoff_date: str | None = None
    supports_reasoning: bool = False
    supports_streaming: bool = False
    supports_parallel_tool_calls: bool = False

    def __post_init__(self) -> None:
        if not isinstance(self.tool_registry, ToolRegistry):
            self.tool_registry = ToolRegistry(self.tool_registry)
        self.subagent_model_overrides = _copy_string_sequence(
            self.subagent_model_overrides,
            field_name="subagent_model_overrides",
        )
        self.capabilities = _copy_mapping(self.capabilities)
        self.provider_options_map = _copy_mapping(self.provider_options_map)
        self.supports_reasoning = bool(
            self.supports_reasoning or self.capabilities.get("reasoning")
        )
        self.supports_streaming = bool(
            self.supports_streaming or self.capabilities.get("streaming")
        )
        self.supports_parallel_tool_calls = bool(
            self.supports_parallel_tool_calls
            or self.capabilities.get("parallel_tool_calls")
        )
        if self.knowledge_cutoff is not None and not isinstance(self.knowledge_cutoff, str):
            raise TypeError("knowledge_cutoff must be a string or None")
        if self.knowledge_cutoff_date is not None and not isinstance(
            self.knowledge_cutoff_date,
            str,
        ):
            raise TypeError("knowledge_cutoff_date must be a string or None")
        if self.knowledge_cutoff is None and self.knowledge_cutoff_date is not None:
            self.knowledge_cutoff = self.knowledge_cutoff_date
        if self.knowledge_cutoff_date is None and self.knowledge_cutoff is not None:
            self.knowledge_cutoff_date = self.knowledge_cutoff

    def build_system_prompt(
        self,
        environment: ExecutionEnvironment,
        project_docs: Mapping[str, Any] | None = None,
    ) -> str:
        return build_layered_system_prompt(self, environment, project_docs)

    def tools(self) -> list[ToolDefinition]:
        return self.tool_registry.definitions()

    def provider_options(self, session_config: Any | None = None) -> dict[str, Any]:
        return dict(self.provider_options_map)

    def supports(self, capability: str) -> bool:
        normalized = _normalize_capability_name(capability)
        if normalized == "reasoning":
            return bool(self.supports_reasoning or self.capabilities.get("reasoning"))
        if normalized == "streaming":
            return bool(self.supports_streaming or self.capabilities.get("streaming"))
        if normalized == "parallel_tool_calls":
            return bool(
                self.supports_parallel_tool_calls
                or self.capabilities.get("parallel_tool_calls")
            )
        return bool(self.capabilities.get(normalized))

    @property
    def capability_flags(self) -> dict[str, bool]:
        return self.capabilities

    @capability_flags.setter
    def capability_flags(self, value: Mapping[str, bool]) -> None:
        self.capabilities = _copy_mapping(value)

    def allows_subagent_model_override(self, requested_model: str) -> bool:
        requested_model = requested_model.strip()
        if not requested_model:
            return False
        if requested_model == self.model:
            return True
        if self.capabilities.get("subagent_model_override"):
            return True
        return any(
            allowed_model == "*" or allowed_model == requested_model
            for allowed_model in self.subagent_model_overrides
        )

    def allowed_subagent_model_overrides(self) -> tuple[str, ...]:
        return tuple(self.subagent_model_overrides)


def _copy_string_sequence(value: Any, *, field_name: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        values = (value,)
    else:
        try:
            values = tuple(value)
        except TypeError as exc:
            raise TypeError(f"{field_name} must be a sequence of strings") from exc
    normalized: list[str] = []
    for item in values:
        if not isinstance(item, str):
            raise TypeError(f"{field_name} must be a sequence of strings")
        item = item.strip()
        if item:
            normalized.append(item)
    return tuple(normalized)


__all__ = ["ProviderProfile"]
