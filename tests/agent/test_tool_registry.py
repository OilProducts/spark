from __future__ import annotations

import pytest

import agent


def _noop_executor(
    arguments: dict[str, object],
    execution_environment: object,
) -> str:
    return "ok"


def test_tool_definition_defaults_to_object_root_json_schema() -> None:
    definition = agent.ToolDefinition(name="lookup", description="Lookup values")

    assert definition.parameters == {"type": "object"}


@pytest.mark.parametrize(
    "parameters",
    [
        {"type": "string"},
        {"type": ["object", "string"]},
        {},
    ],
)
def test_tool_definition_rejects_non_object_root_json_schema(
    parameters: dict[str, object],
) -> None:
    with pytest.raises(ValueError, match="object-root"):
        agent.ToolDefinition(
            name="lookup",
            description="Lookup values",
            parameters=parameters,
        )


def test_registered_tool_requires_an_executor_callable() -> None:
    definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )

    with pytest.raises(TypeError, match="executor"):
        agent.RegisteredTool(definition=definition)


def test_tool_registry_rejects_definition_only_registration_without_executor() -> None:
    registry = agent.ToolRegistry()
    definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )

    with pytest.raises(TypeError, match="executor"):
        registry.register(definition)


def test_tool_registry_latest_registration_wins_and_unregisters_cleanly() -> None:
    first_definition = agent.ToolDefinition(
        name="lookup",
        description="First definition",
        parameters={"type": "object"},
    )
    second_definition = agent.ToolDefinition(
        name="lookup",
        description="Second definition",
        parameters={"type": "object"},
    )

    first_registered = agent.RegisteredTool(
        definition=first_definition,
        executor=_noop_executor,
    )
    second_registered = agent.RegisteredTool(
        definition=second_definition,
        executor=_noop_executor,
    )

    registry = agent.ToolRegistry({"lookup": first_registered})
    registry.register(second_registered)

    assert registry.get("lookup") is second_registered
    assert registry.definitions() == [second_definition]
    assert registry.names() == ["lookup"]

    removed = registry.unregister("lookup")

    assert removed is second_registered
    assert registry.get("lookup") is None
    assert registry.definitions() == []
    assert registry.names() == []


def test_tool_registry_registers_definition_with_latest_wins_behavior() -> None:
    registry = agent.ToolRegistry()

    first_definition = agent.ToolDefinition(
        name="lookup",
        description="First definition",
        parameters={"type": "object"},
    )
    second_definition = agent.ToolDefinition(
        name="lookup",
        description="Second definition",
        parameters={"type": "object"},
    )

    first_registered = registry.register(first_definition, executor=_noop_executor)
    second_registered = registry.register(second_definition, executor=_noop_executor)

    assert first_registered.definition is first_definition
    assert second_registered.definition is second_definition
    assert registry.get("lookup") is second_registered
    assert registry.definitions() == [second_definition]
