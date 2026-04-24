from __future__ import annotations

from collections.abc import Callable, Iterator, Mapping
from dataclasses import dataclass, field, replace
from typing import Any

from .environment import ExecutionEnvironment


def _copy_mapping(mapping: Mapping[str, Any] | None) -> dict[str, Any]:
    if mapping is None:
        return {}
    return dict(mapping)


def _validate_name(value: Any, field_name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{field_name} must be a string")
    if not value.strip():
        raise ValueError(f"{field_name} must not be empty")
    return value


def _validate_description(value: Any) -> str:
    return _validate_name(value, "description")


def _default_object_root_schema() -> dict[str, Any]:
    return {"type": "object"}


def _validate_object_root_schema(parameters: Mapping[str, Any]) -> dict[str, Any]:
    schema = dict(parameters)
    schema_type = schema.get("type")
    if schema_type == "object":
        return schema
    raise ValueError("parameters must describe an object-root JSON Schema")


@dataclass(slots=True)
class ToolDefinition:
    name: str
    description: str
    parameters: dict[str, Any] = field(default_factory=_default_object_root_schema)
    metadata: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.name = _validate_name(self.name, "name")
        self.description = _validate_description(self.description)
        if not isinstance(self.parameters, Mapping):
            raise TypeError("parameters must be a mapping")
        self.parameters = _validate_object_root_schema(self.parameters)
        self.metadata = _copy_mapping(self.metadata)


@dataclass(slots=True)
class ToolOutput:
    content: str | dict[str, Any] | list[Any]
    is_error: bool = False
    image_data: bytes | None = None
    image_media_type: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.content, (str, dict, list)):
            raise TypeError("content must be a string, dict, or list")
        if not isinstance(self.is_error, bool):
            raise TypeError("is_error must be a boolean")
        if self.image_data is not None and not isinstance(self.image_data, bytes):
            raise TypeError("image_data must be bytes or None")
        if self.image_media_type is not None and not isinstance(self.image_media_type, str):
            raise TypeError("image_media_type must be a string or None")


ToolExecutor = Callable[[dict[str, Any], ExecutionEnvironment], Any]


@dataclass(slots=True)
class RegisteredTool:
    definition: ToolDefinition
    executor: ToolExecutor
    metadata: dict[str, Any] = field(default_factory=dict)
    enabled: bool = True

    def __post_init__(self) -> None:
        if not isinstance(self.definition, ToolDefinition):
            raise TypeError("definition must be a ToolDefinition")
        if not callable(self.executor):
            raise TypeError("executor must be callable")
        self.metadata = _copy_mapping(self.metadata)
        if not isinstance(self.enabled, bool):
            raise TypeError("enabled must be a boolean")


class ToolRegistry:
    def __init__(
        self,
        tools: Mapping[str, RegisteredTool] | None = None,
    ) -> None:
        self._tools: dict[str, RegisteredTool] = {}
        for name, tool in dict(tools or {}).items():
            self.register(tool, name=name)

    def register(
        self,
        tool: RegisteredTool | ToolDefinition,
        *,
        name: str | None = None,
        executor: ToolExecutor | None = None,
        metadata: Mapping[str, Any] | None = None,
        enabled: bool | None = None,
    ) -> RegisteredTool:
        if isinstance(tool, ToolDefinition):
            if executor is None or not callable(executor):
                raise TypeError("executor must be callable when registering a ToolDefinition")
            registered = RegisteredTool(
                definition=tool,
                executor=executor,
                metadata=_copy_mapping(metadata),
                enabled=True if enabled is None else enabled,
            )
        elif isinstance(tool, RegisteredTool):
            registered = tool
            if executor is not None:
                if not callable(executor):
                    raise TypeError("executor must be callable")
                registered.executor = executor
            if not callable(registered.executor):
                raise TypeError("executor must be callable")
            if metadata is not None:
                registered.metadata = _copy_mapping(metadata)
            if enabled is not None:
                if not isinstance(enabled, bool):
                    raise TypeError("enabled must be a boolean")
                registered.enabled = enabled
        else:
            raise TypeError("tool must be a ToolDefinition or RegisteredTool")

        tool_name = name or registered.definition.name
        tool_name = _validate_name(tool_name, "name")
        if tool_name != registered.definition.name:
            registered.definition = replace(registered.definition, name=tool_name)

        self._tools[tool_name] = registered
        return registered

    def unregister(self, name: str) -> RegisteredTool | None:
        return self._tools.pop(name, None)

    def get(self, name: str) -> RegisteredTool | None:
        return self._tools.get(name)

    def definitions(self) -> list[ToolDefinition]:
        return [tool.definition for tool in self._tools.values()]

    def names(self) -> list[str]:
        return list(self._tools)

    def items(self) -> list[tuple[str, RegisteredTool]]:
        return list(self._tools.items())

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and name in self._tools

    def __iter__(self) -> Iterator[str]:
        return iter(self._tools)

    def __len__(self) -> int:
        return len(self._tools)


__all__ = ["RegisteredTool", "ToolDefinition", "ToolOutput", "ToolRegistry"]
