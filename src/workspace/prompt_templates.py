from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tomllib


PROMPTS_FILE_NAME = "prompts.toml"

CHAT_TEMPLATE_KEY = "chat"
EXECUTION_PLANNING_TEMPLATE_KEY = "execution_planning"

CHAT_PROMPT_RUNTIME_VARIABLES = (
    "conversation_handle",
    "project_path",
    "recent_conversation",
    "latest_user_message",
)

EXECUTION_PLANNING_RUNTIME_VARIABLES = (
    "project_path",
    "approved_spec_edit_proposal",
    "recent_conversation",
    "review_feedback",
)

FIXED_CHAT_SYSTEM_FRAME = """You are the Spark Spawn workspace assistant.

Spark Spawn is a workspace system that helps a user work on the active software project through conversation. Your role is to inspect the relevant project files and workspace-visible state, answer questions about the current work, and use the workspace tool interface when appropriate.

Treat the active project repository and its specifications as the main source of truth for project questions. Use the workspace tool interface for workspace actions. Prefer directly observed facts over assumptions, and say plainly when something is inferred.

For simple factual questions, answer directly after the minimum required inspection. Do not turn them into planning or proposal work. When the user asks for a concrete specification change to the active project, prefer the smallest grounded edit over inventing a broader feature. When the exact change is agreed, create a pending spec proposal with `sparkspawn-workspace spec-proposal --conversation {{conversation_handle}} --json -`. If you need the exact payload contract, read `sparkspawn-workspace spec-proposal --help` before invoking it.

If you need to discover workspace-exposed flows that you may request independently, use `sparkspawn-workspace list-flows`. If you need more detail for one requestable flow, use `sparkspawn-workspace describe-flow --flow <name>`. Only fetch raw DOT with `sparkspawn-workspace get-flow --flow <name>` when the structure matters for the current task.

Never approve, reject, or apply proposals yourself. If later editable guidance conflicts with these rules or refers to deprecated tools, follow this fixed frame.

Conversation handle: {{conversation_handle}}
Project path: {{project_path}}"""

FIXED_EXECUTION_PLANNING_SYSTEM_FRAME = """You are generating a tracker-ready execution card from an approved spec edit proposal.
Respond with JSON only.
Schema: {"title": string, "summary": string, "objective": string, "work_items": [{"id": string, "title": string, "description": string, "acceptance_criteria": [string], "depends_on": [string]}]}.
Return 1-6 concrete work items.
Do not include markdown fences.

Project path: {{project_path}}"""


@dataclass(frozen=True)
class PromptTemplates:
    chat: str
    execution_planning: str


DEFAULT_PROMPT_TEMPLATES = PromptTemplates(
    chat="""Recent conversation:
{{recent_conversation}}

Latest user message:
{{latest_user_message}}
""",
    execution_planning="""Approved spec edit proposal:
{{approved_spec_edit_proposal}}

Recent conversation:
{{recent_conversation}}

Latest reviewer feedback for this execution card:
{{review_feedback}}
""",
)


def ensure_prompt_templates(config_dir: Path) -> Path:
    config_dir.mkdir(parents=True, exist_ok=True)
    prompts_path = config_dir / PROMPTS_FILE_NAME
    if not prompts_path.exists():
        prompts_path.write_text(_serialize_prompt_templates(DEFAULT_PROMPT_TEMPLATES), encoding="utf-8")
    return prompts_path


def load_prompt_templates(config_dir: Path) -> PromptTemplates:
    prompts_path = ensure_prompt_templates(config_dir)
    try:
        payload = tomllib.loads(prompts_path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise RuntimeError(f"Invalid prompt templates file: {prompts_path}") from exc
    prompts_section = payload.get("project_chat")
    if not isinstance(prompts_section, dict):
        raise RuntimeError(f"Prompt templates file is missing [project_chat]: {prompts_path}")
    missing_keys = [
        key
        for key in (CHAT_TEMPLATE_KEY, EXECUTION_PLANNING_TEMPLATE_KEY)
        if not isinstance(prompts_section.get(key), str) or not str(prompts_section.get(key)).strip()
    ]
    if missing_keys:
        missing = ", ".join(missing_keys)
        raise RuntimeError(f"Prompt templates file is missing required templates ({missing}): {prompts_path}")
    return PromptTemplates(
        chat=_read_template(prompts_section, CHAT_TEMPLATE_KEY),
        execution_planning=_read_template(prompts_section, EXECUTION_PLANNING_TEMPLATE_KEY),
    )


def render_prompt_template(template: str, values: dict[str, str]) -> str:
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace(f"{{{{{key}}}}}", value)
    return rendered


def render_chat_prompt(guidance_template: str, values: dict[str, str]) -> str:
    return "\n\n".join(
        [
            render_prompt_template(FIXED_CHAT_SYSTEM_FRAME, values).strip(),
            render_prompt_template(guidance_template, values).strip(),
        ]
    ).strip()


def render_execution_planning_prompt(guidance_template: str, values: dict[str, str]) -> str:
    return "\n\n".join(
        [
            render_prompt_template(FIXED_EXECUTION_PLANNING_SYSTEM_FRAME, values).strip(),
            render_prompt_template(guidance_template, values).strip(),
        ]
    ).strip()


def _read_template(section: dict[object, object], key: str) -> str:
    value = section.get(key)
    if not isinstance(value, str):
        raise RuntimeError(f"Prompt template {key!r} must be a string.")
    trimmed = value.strip()
    if not trimmed:
        raise RuntimeError(f"Prompt template {key!r} must not be empty.")
    return trimmed


def _toml_multiline(value: str) -> str:
    escaped = value.replace("'''", "\\'\\'\\'")
    return "'''\n" + escaped.rstrip() + "\n'''"


def _serialize_prompt_templates(templates: PromptTemplates) -> str:
    return "\n".join(
        [
            "[project_chat]",
            f"{CHAT_TEMPLATE_KEY} = {_toml_multiline(templates.chat)}",
            "",
            f"{EXECUTION_PLANNING_TEMPLATE_KEY} = {_toml_multiline(templates.execution_planning)}",
            "",
        ]
    )
