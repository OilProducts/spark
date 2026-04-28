from __future__ import annotations

import pytest

import agent
import agent.profiles as profiles
import agent.profiles.anthropic as anthropic_profile_module
import agent.profiles.gemini as gemini_profile_module
import agent.profiles.openai as openai_profile_module
import agent.profiles.openai_compatible as openai_compatible_profile_module


def _noop_executor(arguments: dict[str, object], execution_environment: object) -> str:
    return "ok"


PROVIDER_CASES = [
    (
        "openai",
        agent.create_openai_profile,
        profiles.OpenAIProviderProfile,
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ],
    ),
    (
        "anthropic",
        agent.create_anthropic_profile,
        profiles.AnthropicProviderProfile,
        [
            "read_file",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ],
    ),
    (
        "gemini",
        agent.create_gemini_profile,
        profiles.GeminiProviderProfile,
        [
            "read_file",
            "read_many_files",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "list_dir",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ],
    ),
    (
        "openrouter",
        agent.create_openrouter_profile,
        profiles.OpenAICompatibleProviderProfile,
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ],
    ),
    (
        "litellm",
        agent.create_litellm_profile,
        profiles.OpenAICompatibleProviderProfile,
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ],
    ),
]


def test_provider_profile_export_is_shared_between_agent_modules() -> None:
    assert agent.ProviderProfile is profiles.ProviderProfile


def test_provider_profile_package_exports_provider_factories_and_registry_builders() -> None:
    assert profiles.create_openai_profile is openai_profile_module.create_openai_profile
    assert profiles.build_openai_tool_registry is openai_profile_module.build_openai_tool_registry
    assert profiles.register_openai_tools is openai_profile_module.register_openai_tools

    assert profiles.create_anthropic_profile is anthropic_profile_module.create_anthropic_profile
    assert (
        profiles.build_anthropic_tool_registry
        is anthropic_profile_module.build_anthropic_tool_registry
    )
    assert profiles.register_anthropic_tools is anthropic_profile_module.register_anthropic_tools

    assert profiles.create_gemini_profile is gemini_profile_module.create_gemini_profile
    assert profiles.build_gemini_tool_registry is gemini_profile_module.build_gemini_tool_registry
    assert profiles.register_gemini_tools is gemini_profile_module.register_gemini_tools

    assert (
        profiles.create_openrouter_profile
        is openai_compatible_profile_module.create_openrouter_profile
    )
    assert profiles.create_litellm_profile is openai_compatible_profile_module.create_litellm_profile


@pytest.mark.parametrize(
    ("provider_name", "factory", "profile_type", "expected_tool_names"),
    PROVIDER_CASES,
)
def test_provider_profiles_expose_provider_specific_tools_and_object_root_schemas(
    provider_name: str,
    factory,
    profile_type,
    expected_tool_names: list[str],
) -> None:
    profile = factory(model={
        "openai": "gpt-5.2",
        "anthropic": "claude-sonnet-4-5",
        "gemini": "gemini-3.1-pro-preview",
        "openrouter": "anthropic/claude-sonnet-4.5",
        "litellm": "team-model",
    }[provider_name])

    assert isinstance(profile, profile_type)
    assert profile.id == provider_name
    assert profile.tool_registry.names() == expected_tool_names
    for tool_name in profile.tool_registry.names():
        definition = profile.tool_registry.get(tool_name).definition
        assert definition.parameters["type"] == "object"


@pytest.mark.parametrize(
    ("provider_name", "factory", "expected_tool_names"),
    [(name, factory, tool_names) for name, factory, _, tool_names in PROVIDER_CASES],
)
def test_provider_profiles_build_requests_through_the_public_session_api(
    provider_name: str,
    factory,
    expected_tool_names: list[str],
) -> None:
    profile = factory(
        model={
            "openai": "gpt-5.2",
            "anthropic": "claude-sonnet-4-5",
            "gemini": "gemini-3.1-pro-preview",
            "openrouter": "anthropic/claude-sonnet-4.5",
            "litellm": "team-model",
        }[provider_name],
    )
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir="."),
    )

    request = session.build_request("Session system prompt")

    assert request.provider == provider_name
    assert request.messages[0].text == "Session system prompt"
    assert [tool.name for tool in request.tools] == expected_tool_names
    assert request.tool_choice is not None
    assert request.tool_choice.mode == "auto"


@pytest.mark.parametrize("factory", [agent.create_openrouter_profile, agent.create_litellm_profile])
def test_openai_compatible_profiles_do_not_inject_openai_reasoning_options(factory) -> None:
    profile = factory(model="explicit-model")
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir="."),
        config=agent.SessionConfig(reasoning_effort="high"),
    )

    request = session.build_request("Session system prompt")

    assert request.provider in {"openrouter", "litellm"}
    assert request.provider_options == {request.provider: {}}


def test_provider_profile_exposes_fields_capabilities_and_copying_behavior(
    tmp_path,
) -> None:
    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )
    registered_tool = agent.RegisteredTool(
        definition=tool_definition,
        executor=_noop_executor,
        metadata={"kind": "builtin"},
    )
    registry = agent.ToolRegistry({"lookup": registered_tool})
    provider_options = {"temperature": 0.2}
    capabilities = {"tool_calls": True}
    profile = agent.ProviderProfile(
        id="fake-provider",
        model="fake-model",
        tool_registry=registry,
        capabilities=capabilities,
        provider_options_map=provider_options,
        context_window_size=4096,
        display_name="Test Provider",
        supports_reasoning=True,
        supports_streaming=False,
        supports_parallel_tool_calls=True,
    )
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)

    provider_options["temperature"] = 0.9
    capabilities["tool_calls"] = False

    assert profile.id == "fake-provider"
    assert profile.model == "fake-model"
    assert profile.display_name == "Test Provider"
    assert profile.tool_registry is registry
    assert profile.tools() == [tool_definition]
    assert profile.provider_options() == {"temperature": 0.2}
    assert profile.provider_options_map == {"temperature": 0.2}
    assert profile.capability_flags == {"tool_calls": True}
    assert profile.supports("tool_calls") is True
    assert profile.supports("reasoning") is True
    assert profile.supports("supports_reasoning") is True
    assert profile.supports_reasoning is True
    assert profile.supports_streaming is False
    assert profile.supports_parallel_tool_calls is True
    assert profile.context_window_size == 4096
    request = agent.Session(
        profile=profile,
        execution_env=environment,
    ).build_request("System prompt")
    assert request.provider == "fake-provider"
    assert request.messages[0].text == "System prompt"
    assert [tool.name for tool in request.tools] == ["lookup"]
    assert request.provider_options == {"fake-provider": {"temperature": 0.2}}

    returned_options = profile.provider_options()
    returned_options["temperature"] = 0.7
    assert profile.provider_options() == {"temperature": 0.2}


def test_provider_profile_prompt_hook_is_overridable_and_receives_inputs(
    tmp_path,
) -> None:
    captured: dict[str, object] = {}

    class PromptProfile(profiles.ProviderProfile):
        def build_system_prompt(self, environment, project_docs):
            captured["environment"] = environment
            captured["project_docs"] = dict(project_docs)
            return f"{environment.working_directory()}::{','.join(sorted(project_docs))}"

    profile = PromptProfile(id="hook-provider", model="hook-model")
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    project_docs = {"README.md": "readme", "AGENTS.md": "docs"}

    assert profile.build_system_prompt(environment, project_docs) == (
        f"{tmp_path}::AGENTS.md,README.md"
    )
    assert captured["environment"] is environment
    assert captured["project_docs"] == project_docs


def test_provider_profile_tool_registry_uses_latest_wins_after_creation() -> None:
    profile = agent.ProviderProfile(id="provider", model="model")
    first_definition = agent.ToolDefinition(
        name="lookup",
        description="First tool",
        parameters={"type": "object"},
    )
    second_definition = agent.ToolDefinition(
        name="lookup",
        description="Second tool",
        parameters={"type": "object"},
    )

    profile.tool_registry.register(first_definition, executor=_noop_executor)
    profile.tool_registry.register(second_definition, executor=_noop_executor)

    assert profile.tools() == [second_definition]
    assert profile.tool_registry.get("lookup").definition is second_definition
