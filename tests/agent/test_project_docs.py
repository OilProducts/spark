from __future__ import annotations

import pytest

import agent
from agent import project_docs


def _initialize_git_repo(
    environment: agent.LocalExecutionEnvironment,
    working_dir: str | object | None = None,
) -> None:
    assert (
        environment.exec_command(
            "git init",
            working_dir=working_dir or environment.working_directory(),
        ).exit_code
        == 0
    )


@pytest.mark.parametrize(
    ("model", "expected_paths", "expected_contents"),
    [
        (
            "gpt-5.2",
            [
                "AGENTS.md",
                ".codex/instructions.md",
                "nested/AGENTS.md",
                "nested/.codex/instructions.md",
            ],
            [
                "root agents",
                "root openai",
                "nested agents",
                "nested openai",
            ],
        ),
        (
            "claude-sonnet-4-5",
            [
                "AGENTS.md",
                "CLAUDE.md",
                "nested/AGENTS.md",
                "nested/CLAUDE.md",
            ],
            [
                "root agents",
                "root claude",
                "nested agents",
                "nested claude",
            ],
        ),
        (
            "gemini-3.1-pro-preview",
            [
                "AGENTS.md",
                "GEMINI.md",
                "nested/AGENTS.md",
                "nested/GEMINI.md",
            ],
            [
                "root agents",
                "root gemini",
                "nested agents",
                "nested gemini",
            ],
        ),
    ],
)
def test_discover_project_documents_filters_by_provider_and_keeps_root_to_leaf_order(
    tmp_path,
    model: str,
    expected_paths: list[str],
    expected_contents: list[str],
) -> None:
    nested_working_dir = tmp_path / "nested"
    nested_working_dir.mkdir()
    environment = agent.LocalExecutionEnvironment(working_dir=nested_working_dir)
    _initialize_git_repo(environment, working_dir=tmp_path)

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

    profile = agent.ProviderProfile(id="provider", model=model)
    bundle = project_docs.discover_project_documents(environment, profile)

    assert isinstance(bundle, project_docs.ProjectDocuments)
    assert [document.path for document in bundle] == expected_paths
    assert [document.content for document in bundle] == expected_contents
    assert bundle.truncated is False


def test_project_documents_rendering_appends_the_truncation_marker_when_budget_overflows(
    tmp_path,
) -> None:
    nested_working_dir = tmp_path / "nested"
    nested_working_dir.mkdir()
    environment = agent.LocalExecutionEnvironment(working_dir=nested_working_dir)
    _initialize_git_repo(environment, working_dir=tmp_path)

    environment.write_file(tmp_path / "AGENTS.md", "intro\n" + ("A" * 33000) + "\nroot-end")
    environment.write_file(nested_working_dir / "AGENTS.md", "nested guidance")

    profile = agent.ProviderProfile(id="provider", model="gpt-5.2")
    bundle = project_docs.discover_project_documents(environment, profile)
    rendered = project_docs.render_project_documents(bundle)

    assert bundle.truncated is True
    assert len(bundle) == 1
    assert bundle[0].path == "AGENTS.md"
    assert bundle[0].truncated is True
    assert project_docs.PROJECT_INSTRUCTION_TRUNCATION_MARKER in rendered
    assert "root-end" not in rendered
    assert "nested guidance" not in rendered
    assert rendered.startswith("### AGENTS.md\nintro")
