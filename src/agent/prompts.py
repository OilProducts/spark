from __future__ import annotations

from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from datetime import date
from typing import Any

from unified_llm.models import get_model_info

from . import project_docs as project_docs_module
from .environment import ExecutionEnvironment
from .project_docs import ProjectDocument, ProjectDocuments


def _provider_family(profile: Any) -> str | None:
    model = getattr(profile, "model", None)
    if isinstance(model, str) and model:
        model_info = get_model_info(model)
        if model_info is not None:
            return model_info.provider.casefold()

    probe_values = [
        getattr(profile, "id", None),
        getattr(profile, "model", None),
        getattr(profile, "display_name", None),
    ]
    haystack = " ".join(value for value in probe_values if isinstance(value, str)).casefold()
    if "anthropic" in haystack or "claude" in haystack:
        return "anthropic"
    if "gemini" in haystack:
        return "gemini"
    if "openai" in haystack or haystack.startswith(("gpt-", "o1", "o3")):
        return "openai"
    return None


def _display_name(profile: Any) -> str:
    display_name = getattr(profile, "display_name", None)
    if isinstance(display_name, str) and display_name.strip():
        return display_name

    model = getattr(profile, "model", None)
    if isinstance(model, str) and model:
        model_info = get_model_info(model)
        if model_info is not None:
            return model_info.display_name
        return model
    return "unknown"


def _knowledge_cutoff(profile: Any) -> str:
    for attribute in ("knowledge_cutoff_date", "knowledge_cutoff"):
        value = getattr(profile, attribute, None)
        if isinstance(value, str) and value.strip():
            return value
    return "unknown"


def _exec_command_output(
    environment: ExecutionEnvironment,
    command: str,
    working_directory: str,
) -> str | None:
    try:
        result = environment.exec_command(command, working_dir=working_directory)
    except Exception:
        return None

    if result.exit_code != 0:
        return None
    output = result.stdout.strip()
    return output or None


def _exec_command_candidates(
    environment: ExecutionEnvironment,
    command: str,
    working_directory: str,
) -> str | None:
    candidates: list[str] = []
    for candidate in (
        working_directory,
        str(project_docs_module._resolve_path_text(working_directory)),
    ):
        if candidate not in candidates:
            candidates.append(candidate)

    for candidate in candidates:
        output = _exec_command_output(environment, command, candidate)
        if output is not None:
            return output
    return None


def _git_status_counts(
    environment: ExecutionEnvironment,
    working_directory: str,
) -> tuple[int, int]:
    status_output = _exec_command_candidates(
        environment,
        "git status --porcelain=v1",
        working_directory,
    )
    if not status_output:
        return 0, 0

    modified = 0
    untracked = 0
    for line in status_output.splitlines():
        if line.startswith("??"):
            untracked += 1
        elif line.strip():
            modified += 1
    return modified, untracked


def _recent_commit_messages(
    environment: ExecutionEnvironment,
    working_directory: str,
) -> list[str]:
    output = _exec_command_candidates(
        environment,
        "git log -n 5 --pretty=format:%s",
        working_directory,
    )
    if not output:
        return []
    return [line.strip() for line in output.splitlines() if line.strip()]


def _current_branch(environment: ExecutionEnvironment, working_directory: str) -> str:
    branch = _exec_command_candidates(
        environment,
        "git branch --show-current",
        working_directory,
    )
    if branch:
        return branch
    abbrev = _exec_command_candidates(
        environment,
        "git rev-parse --abbrev-ref HEAD",
        working_directory,
    )
    if abbrev:
        return abbrev
    return "unknown"


def _is_git_repository(environment: ExecutionEnvironment, working_directory: str) -> bool:
    output = _exec_command_candidates(
        environment,
        "git rev-parse --is-inside-work-tree",
        working_directory,
    )
    return output == "true"


@dataclass(slots=True)
class EnvironmentContext:
    working_directory: str
    is_git_repository: bool
    current_branch: str
    modified_count: int
    untracked_count: int
    recent_commit_messages: list[str]
    platform: str
    os_version: str
    today: str
    model_display_name: str
    knowledge_cutoff: str


def snapshot_environment_context(
    profile: Any,
    environment: ExecutionEnvironment,
) -> EnvironmentContext:
    working_directory = environment.working_directory()
    is_git_repository = _is_git_repository(environment, working_directory)
    if is_git_repository:
        current_branch = _current_branch(environment, working_directory)
        modified_count, untracked_count = _git_status_counts(environment, working_directory)
        recent_commit_messages = _recent_commit_messages(environment, working_directory)
    else:
        current_branch = "unknown"
        modified_count = 0
        untracked_count = 0
        recent_commit_messages = []

    return EnvironmentContext(
        working_directory=working_directory,
        is_git_repository=is_git_repository,
        current_branch=current_branch,
        modified_count=modified_count,
        untracked_count=untracked_count,
        recent_commit_messages=recent_commit_messages,
        platform=environment.platform(),
        os_version=environment.os_version(),
        today=date.today().isoformat(),
        model_display_name=_display_name(profile),
        knowledge_cutoff=_knowledge_cutoff(profile),
    )


def build_environment_context_block(context: EnvironmentContext) -> str:
    lines = [
        "<environment>",
        f"Working directory: {context.working_directory}",
        f"Is git repository: {str(context.is_git_repository).lower()}",
        f"Git branch: {context.current_branch}",
        f"Modified files: {context.modified_count}",
        f"Untracked files: {context.untracked_count}",
    ]
    if context.recent_commit_messages:
        lines.append("Recent commit messages:")
        lines.extend(f"- {message}" for message in context.recent_commit_messages)
    else:
        lines.append("Recent commit messages: none")
    lines.extend(
        [
            f"Platform: {context.platform}",
            f"OS version: {context.os_version}",
            f"Today's date: {context.today}",
            f"Model: {context.model_display_name}",
            f"Knowledge cutoff: {context.knowledge_cutoff}",
            "</environment>",
        ]
    )
    return "\n".join(lines)


def _provider_identity_line(provider_family: str | None) -> str:
    return {
        "openai": "Provider identity: OpenAI coding agent.",
        "anthropic": "Provider identity: Anthropic coding agent.",
        "gemini": "Provider identity: Gemini coding agent.",
    }.get(provider_family, "Provider identity: Unified LLM coding agent.")


def _provider_tool_usage_line(provider_family: str | None) -> str:
    if provider_family == "openai":
        return (
            "Tool usage: inspect before editing, use the available tools deliberately, "
            "and prefer apply_patch for targeted edits."
        )
    if provider_family == "anthropic":
        return (
            "Tool usage: read before edit, use read_file to inspect existing files, "
            "prefer edit_file for targeted replacements, use write_file for new files "
            "or full rewrites, and keep the smallest tool action that solves the task."
        )
    if provider_family == "gemini":
        return (
            "Tool usage: inspect before editing, use tools intentionally, and keep "
            "changes focused."
        )
    return (
        "Tool usage: inspect before editing, use the available tools deliberately, "
        "and keep the request path concise."
    )


def _provider_edit_guidance_line(provider_family: str | None) -> str:
    if provider_family == "openai":
        return (
            "Edit guidance: prefer apply_patch for targeted edits, use the "
            "*** Begin Patch / *** End Patch format for patch-based edits, avoid "
            "rewriting entire files when a smaller patch is enough, and keep diffs "
            "minimal."
        )
    if provider_family == "anthropic":
        return (
            "Edit guidance: prefer edit_file for targeted replacements, use "
            "write_file for new files or complete rewrites, and ensure the old_string "
            "is unique when replacing file content."
        )
    if provider_family == "gemini":
        return (
            "Edit guidance: prefer patch-style edits over full file rewrites and keep "
            "the change set narrow."
        )
    return (
        "Edit guidance: prefer patch-style edits over full file rewrites and keep "
        "the change set narrow."
    )


def _provider_project_instruction_line(provider_family: str | None) -> str:
    if provider_family == "openai":
        return (
            "Project instruction conventions: AGENTS.md is always loaded; deeper "
            "project docs override shallower ones; .codex/instructions.md is "
            "OpenAI-only."
        )
    if provider_family == "anthropic":
        return (
            "Project instruction conventions: AGENTS.md is always loaded; deeper "
            "project docs override shallower ones; CLAUDE.md is Anthropic-only."
        )
    if provider_family == "gemini":
        return (
            "Project instruction conventions: AGENTS.md is always loaded; deeper "
            "project docs override shallower ones; GEMINI.md is Gemini-only."
        )
    return (
        "Project instruction conventions: AGENTS.md is always loaded and deeper "
        "project docs override shallower ones."
    )


def _provider_coding_guidance_line(provider_family: str | None) -> str:
    if provider_family == "openai":
        return (
            "Coding guidance: preserve observable behavior, update tests when "
            "behavior changes, and do not vendor external CLI prompt text."
        )
    if provider_family == "anthropic":
        return (
            "Coding guidance: preserve observable behavior, update tests when "
            "behavior changes, and keep edits readable and minimal."
        )
    if provider_family == "gemini":
        return (
            "Coding guidance: preserve observable behavior, update tests when "
            "behavior changes, and keep edits readable and minimal."
        )
    return (
        "Coding guidance: preserve observable behavior, update tests when behavior "
        "changes, and keep edits readable and minimal."
    )


def build_provider_base_instructions(profile: Any) -> str:
    provider_family = _provider_family(profile)
    lines = [
        "<provider_base_instructions>",
        _provider_identity_line(provider_family),
        _provider_tool_usage_line(provider_family),
        _provider_edit_guidance_line(provider_family),
        _provider_project_instruction_line(provider_family),
        _provider_coding_guidance_line(provider_family),
        "</provider_base_instructions>",
    ]
    return "\n".join(lines)


def build_tool_descriptions(profile: Any) -> str:
    tools = list(getattr(profile, "tools", lambda: [])())
    lines = ["<tools>"]
    if not tools:
        lines.append("No tools are registered for this profile.")
    else:
        for tool in tools:
            name = getattr(tool, "name", "unknown")
            description = getattr(tool, "description", "")
            lines.append(f"- {name}: {description}")
            parameters = getattr(tool, "parameters", None)
            if parameters is not None:
                lines.append(f"  Parameters: {parameters}")
            metadata = getattr(tool, "metadata", None)
            if metadata:
                lines.append(f"  Metadata: {metadata}")
    lines.append("</tools>")
    return "\n".join(lines)


def _render_project_documents(
    project_documents: ProjectDocument
    | ProjectDocuments
    | Mapping[str, Any]
    | Iterable[Any]
    | None,
    *,
    environment: ExecutionEnvironment,
    profile: Any,
) -> str:
    if project_documents is None:
        return project_docs_module.load_project_documents(environment, profile)
    return project_docs_module.render_project_documents(project_documents)


def _render_user_overrides(user_overrides: str | Iterable[str] | None) -> str:
    if user_overrides is None:
        return ""
    if isinstance(user_overrides, str):
        text = user_overrides
    else:
        text = "\n".join(str(item) for item in user_overrides)
    if not text.strip():
        return ""
    return "\n".join(["<user_overrides>", text, "</user_overrides>"])


def build_system_prompt(
    profile: Any,
    environment: ExecutionEnvironment,
    project_docs: ProjectDocument
    | ProjectDocuments
    | Mapping[str, Any]
    | Iterable[Any]
    | None = None,
    *,
    user_overrides: str | Iterable[str] | None = None,
) -> str:
    layers = [
        build_provider_base_instructions(profile),
        build_environment_context_block(snapshot_environment_context(profile, environment)),
        build_tool_descriptions(profile),
    ]

    project_documents = _render_project_documents(
        project_docs,
        environment=environment,
        profile=profile,
    )
    if project_documents:
        layers.append(
            "\n".join(
                [
                    "<project_instructions>",
                    project_documents,
                    "</project_instructions>",
                ]
            )
        )

    user_overrides_text = _render_user_overrides(user_overrides)
    if user_overrides_text:
        layers.append(user_overrides_text)

    return "\n\n".join(layers)


__all__ = [
    "EnvironmentContext",
    "build_environment_context_block",
    "build_provider_base_instructions",
    "build_system_prompt",
    "build_tool_descriptions",
    "snapshot_environment_context",
]
