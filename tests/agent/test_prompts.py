from __future__ import annotations

import asyncio
from datetime import date

import pytest

import unified_llm
import unified_llm.agent as agent
from unified_llm.agent import prompts


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


def _initialize_git_repo(environment: agent.LocalExecutionEnvironment) -> str:
    init_result = environment.exec_command(
        "git init",
        working_dir=environment.working_directory(),
    )
    assert init_result.exit_code == 0
    assert (
        environment.exec_command(
            'git config user.name "Test User"',
            working_dir=environment.working_directory(),
        ).exit_code
        == 0
    )
    assert (
        environment.exec_command(
            'git config user.email "test@example.com"',
            working_dir=environment.working_directory(),
        ).exit_code
        == 0
    )
    environment.write_file("tracked.txt", "tracked\n")
    assert (
        environment.exec_command(
            "git add tracked.txt",
            working_dir=environment.working_directory(),
        ).exit_code
        == 0
    )
    assert (
        environment.exec_command(
            'git commit -m "Initial commit"',
            working_dir=environment.working_directory(),
        ).exit_code
        == 0
    )
    branch = environment.exec_command(
        "git branch --show-current",
        working_dir=environment.working_directory(),
    ).stdout.strip()
    if not branch:
        branch = environment.exec_command(
            "git rev-parse --abbrev-ref HEAD",
            working_dir=environment.working_directory(),
        ).stdout.strip()
    return branch or "unknown"


def test_build_system_prompt_assembles_layers_and_snapshots_environment_at_session_start(
    tmp_path,
) -> None:
    environment = agent.LocalExecutionEnvironment(working_dir=tmp_path)
    branch = _initialize_git_repo(environment)
    environment.write_file("tracked.txt", "tracked\nmodified\n")
    environment.write_file("untracked.txt", "new\n")

    tool_definition = agent.ToolDefinition(
        name="lookup",
        description="Lookup values",
        parameters={"type": "object"},
    )
    profile = agent.ProviderProfile(
        id="openai-profile",
        model="gpt-5.2",
        display_name="GPT-5.2",
        knowledge_cutoff="2024-06",
    )
    profile.tool_registry.register(tool_definition, executor=lambda arguments, env: "ok")

    prompt = prompts.build_system_prompt(
        profile,
        environment,
        {
            "AGENTS.md": "Root guidance",
            "nested/AGENTS.md": "Nested guidance",
        },
        user_overrides="User override guidance",
    )

    section_positions = [
        prompt.index("<provider_base_instructions>"),
        prompt.index("<environment>"),
        prompt.index("<tools>"),
        prompt.index("<project_instructions>"),
        prompt.index("<user_overrides>"),
    ]
    assert section_positions == sorted(section_positions)

    assert "Provider identity:" in prompt
    assert "Tool usage:" in prompt
    assert "Edit guidance:" in prompt
    assert "*** Begin Patch / *** End Patch format" in prompt
    assert "Project instruction conventions:" in prompt
    assert "Coding guidance:" in prompt
    assert "apply_patch" in prompt
    assert "Working directory: " in prompt
    assert f"Working directory: {tmp_path}" in prompt
    assert "Is git repository: true" in prompt
    assert f"Git branch: {branch}" in prompt
    assert "Modified files: 1" in prompt
    assert "Untracked files: 1" in prompt
    assert "Recent commit messages:" in prompt
    assert "Initial commit" in prompt
    assert f"Platform: {environment.platform()}" in prompt
    assert f"OS version: {environment.os_version()}" in prompt
    assert f"Today's date: {date.today().isoformat()}" in prompt
    assert "Model: GPT-5.2" in prompt
    assert "Knowledge cutoff: 2024-06" in prompt
    assert "lookup" in prompt
    assert "Lookup values" in prompt
    assert "Root guidance" in prompt
    assert "Nested guidance" in prompt
    assert "User override guidance" in prompt


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
    environment.working_directory_value = "mutated-working-directory"

    stream = session.events()
    start_event = await asyncio.wait_for(anext(stream), timeout=1)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Hello")

    request = client.requests[0]
    assert request.messages[0].role == unified_llm.Role.SYSTEM
    system_prompt = request.messages[0].text
    assert "Working directory: initial-working-directory" in system_prompt
    assert "Working directory: mutated-working-directory" not in system_prompt
