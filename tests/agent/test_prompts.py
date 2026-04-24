from __future__ import annotations

import asyncio
from datetime import date
from pathlib import Path

import pytest

import agent
import unified_llm
from agent import project_docs, prompts


class _FakeClient:
    def __init__(self, responses: list[unified_llm.Response]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._responses = list(responses)

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        if not self._responses:
            raise AssertionError("unexpected complete call")
        return self._responses.pop(0)


class _MutableEnvironment:
    def __init__(self, working_directory: str) -> None:
        self.working_directory_value = working_directory

    def read_file(
        self,
        path: str | object,
        offset: int | None = None,
        limit: int | None = None,
    ) -> str:
        raise FileNotFoundError(path)

    def write_file(self, path: str | object, content: str) -> None:
        raise AssertionError("write_file should not be called")

    def file_exists(self, path: str | object) -> bool:
        return False

    def is_directory(self, path: str | object) -> bool:
        return False

    def delete_file(self, path: str | object) -> None:
        return None

    def rename_file(self, source_path: str | object, destination_path: str | object) -> None:
        return None

    def list_directory(self, path: str | object, depth: int) -> list[agent.DirEntry]:
        return []

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | object | None = None,
        env_vars: dict[str, str] | None = None,
    ) -> agent.ExecResult:
        return agent.ExecResult(stdout="", stderr="", exit_code=1, timed_out=False, duration_ms=1)

    def grep(self, pattern: str, path: str | object, options: agent.GrepOptions) -> str:
        return ""

    def glob(self, pattern: str, path: str | object) -> list[str]:
        return []

    def initialize(self) -> None:
        return None

    def cleanup(self) -> None:
        return None

    def working_directory(self) -> str:
        return self.working_directory_value

    def platform(self) -> str:
        return "test-platform"

    def os_version(self) -> str:
        return "test-os"


def _initialize_git_repo(
    environment: agent.LocalExecutionEnvironment,
    working_dir: str | object | None = None,
) -> str:
    repo_dir = Path(working_dir or environment.working_directory())
    init_result = environment.exec_command(
        "git init",
        working_dir=repo_dir,
    )
    assert init_result.exit_code == 0
    assert (
        environment.exec_command(
            'git config user.name "Test User"',
            working_dir=repo_dir,
        ).exit_code
        == 0
    )
    assert (
        environment.exec_command(
            'git config user.email "test@example.com"',
            working_dir=repo_dir,
        ).exit_code
        == 0
    )
    environment.write_file(repo_dir / "tracked.txt", "tracked\n")
    assert (
        environment.exec_command(
            "git add tracked.txt",
            working_dir=repo_dir,
        ).exit_code
        == 0
    )
    assert (
        environment.exec_command(
            'git commit -m "Initial commit"',
            working_dir=repo_dir,
        ).exit_code
        == 0
    )
    branch = environment.exec_command(
        "git branch --show-current",
        working_dir=repo_dir,
    ).stdout.strip()
    if not branch:
        branch = environment.exec_command(
            "git rev-parse --abbrev-ref HEAD",
            working_dir=repo_dir,
        ).stdout.strip()
    return branch or "unknown"


def _register_lookup_tool(profile: agent.ProviderProfile) -> None:
    profile.tool_registry.register(
        agent.ToolDefinition(
            name="lookup",
            description="Lookup values",
            parameters={"type": "object"},
        ),
        executor=lambda arguments, env: "ok",
    )


@pytest.mark.parametrize(
    ("model", "display_name", "included_contents", "excluded_contents"),
    [
        (
            "gpt-5.2",
            "GPT-5.2",
            ("root agents", "nested agents", "root openai", "nested openai"),
            ("root claude", "nested claude", "root gemini", "nested gemini"),
        ),
        (
            "claude-sonnet-4-5",
            "Claude Sonnet 4.5",
            ("root agents", "nested agents", "root claude", "nested claude"),
            ("root openai", "nested openai", "root gemini", "nested gemini"),
        ),
        (
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro",
            ("root agents", "nested agents", "root gemini", "nested gemini"),
            ("root openai", "nested openai", "root claude", "nested claude"),
        ),
    ],
)
def test_build_system_prompt_orders_layers_and_filters_project_documents_by_provider(
    tmp_path,
    model: str,
    display_name: str,
    included_contents: tuple[str, ...],
    excluded_contents: tuple[str, ...],
) -> None:
    nested_working_dir = tmp_path / "nested"
    nested_working_dir.mkdir()
    environment = agent.LocalExecutionEnvironment(working_dir=nested_working_dir)
    branch = _initialize_git_repo(environment, working_dir=tmp_path)
    environment.write_file(tmp_path / "tracked.txt", "tracked\nmodified\n")
    environment.write_file(nested_working_dir / "untracked.txt", "new\n")
    environment.write_file(tmp_path / "AGENTS.md", "root agents")
    environment.write_file(tmp_path / ".codex/instructions.md", "root openai")
    environment.write_file(tmp_path / "CLAUDE.md", "root claude")
    environment.write_file(tmp_path / "GEMINI.md", "root gemini")
    environment.write_file(nested_working_dir / "AGENTS.md", "nested agents")
    environment.write_file(
        nested_working_dir / ".codex/instructions.md",
        "nested openai",
    )
    environment.write_file(nested_working_dir / "CLAUDE.md", "nested claude")
    environment.write_file(nested_working_dir / "GEMINI.md", "nested gemini")

    profile = agent.ProviderProfile(
        id="openai-profile",
        model=model,
        display_name=display_name,
        knowledge_cutoff="2024-06",
    )
    _register_lookup_tool(profile)

    context = prompts.snapshot_environment_context(profile, environment)
    prompt = prompts.build_system_prompt(
        profile,
        environment,
        user_overrides="User override guidance",
    )
    provider_block = prompts.build_provider_base_instructions(profile)
    environment_block = prompts.build_environment_context_block(context)
    tools_block = prompts.build_tool_descriptions(profile)
    project_block = project_docs.load_project_documents(environment, profile)

    assert context.working_directory == str(nested_working_dir)
    assert context.is_git_repository is True
    assert context.current_branch == branch
    assert context.modified_count == 1
    assert context.untracked_count >= 1
    assert context.recent_commit_messages == ["Initial commit"]
    assert context.platform == environment.platform()
    assert context.os_version == environment.os_version()
    assert context.today == date.today().isoformat()
    assert context.model_display_name == display_name
    assert context.knowledge_cutoff == "2024-06"

    assert prompt.index(provider_block) == 0
    assert prompt.index(environment_block) > prompt.index(provider_block)
    assert prompt.index(tools_block) > prompt.index(environment_block)
    assert prompt.index(project_block) > prompt.index(tools_block)
    assert prompt.index("User override guidance") > prompt.index(project_block)

    assert "lookup" in tools_block
    assert "Lookup values" in tools_block
    assert project_block in prompt
    for content in included_contents:
        assert content in project_block
    for content in excluded_contents:
        assert content not in project_block


@pytest.mark.asyncio
async def test_session_caches_the_initial_system_prompt_snapshot() -> None:
    environment = _MutableEnvironment("initial-working-directory")
    client = _FakeClient(
        [
            unified_llm.Response(
                id="resp-1",
                model="fake-model",
                provider="fake-provider",
                message=unified_llm.Message.assistant("Done"),
                finish_reason=unified_llm.FinishReason.STOP,
            )
        ]
    )
    profile = agent.ProviderProfile(
        id="provider",
        model="fake-model",
        display_name="Session Model",
    )
    session = agent.Session(
        profile=profile,
        execution_environment=environment,
        llm_client=client,
    )
    expected_prompt = prompts.build_system_prompt(
        profile,
        _MutableEnvironment("initial-working-directory"),
    )
    environment.working_directory_value = "mutated-working-directory"

    stream = session.events()
    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Hello")

    request = client.requests[0]
    assert request.messages[0].role == unified_llm.Role.SYSTEM
    system_prompt = request.messages[0].text
    assert system_prompt == expected_prompt
    assert "initial-working-directory" in system_prompt
    assert "mutated-working-directory" not in system_prompt
