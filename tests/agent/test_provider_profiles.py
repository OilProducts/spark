from __future__ import annotations

import unified_llm.agent as agent
import unified_llm.agent.profiles as profiles
import unified_llm.agent.profiles.anthropic as anthropic_profile_module
import unified_llm.agent.profiles.gemini as gemini_profile_module
import unified_llm.agent.profiles.openai as openai_profile_module


def _noop_executor(arguments: dict[str, object], execution_environment: object) -> str:
    return "ok"


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
    prompt = profile.build_system_prompt(environment, {"AGENTS.md": "docs"})
    assert "<provider_base_instructions>" in prompt
    assert "<environment>" in prompt
    assert "<tools>" in prompt
    assert "<project_instructions>" in prompt
    assert "Provider identity:" in prompt
    assert "Tool usage:" in prompt
    assert "Edit guidance:" in prompt
    assert "Project instruction conventions:" in prompt
    assert "Coding guidance:" in prompt
    assert "Working directory:" in prompt
    assert "Is git repository: false" in prompt
    assert "AGENTS.md" in prompt
    assert "docs" in prompt

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
