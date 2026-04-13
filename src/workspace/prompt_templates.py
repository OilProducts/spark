from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tomllib


PROMPTS_FILE_NAME = "prompts.toml"

CHAT_TEMPLATE_KEY = "chat"

CHAT_PROMPT_RUNTIME_VARIABLES = (
    "conversation_handle",
    "project_path",
    "flow_library_path",
    "dot_authoring_guide_path",
    "spark_operations_guide_path",
    "flow_validation_command",
    "recent_conversation",
    "latest_user_message",
)

FIXED_CHAT_SYSTEM_FRAME = """You are the Spark workspace assistant.

Spark is a workspace system that helps a user work on the active software project through conversation. Your role is to inspect the relevant project files and workspace-visible state, answer questions about the current work, and use the workspace tool interface when appropriate.

Treat the active project repository and its specifications as the main source of truth for project questions. Use the workspace tool interface for workspace actions. Prefer directly observed facts over assumptions, and say plainly when something is inferred.

For simple factual questions, answer directly after the minimum required inspection. Do not turn them into planning theater or workflow artifacts. When the user asks for a concrete specification change to the active project, prefer the smallest grounded edit over inventing a broader feature.

If you need to discover workspace-exposed flows that you may request independently, use `spark flow list`. If you need more detail for one requestable flow, use `spark flow describe --flow <name>`. Only fetch raw DOT with `spark flow get --flow <name>` when the structure matters for the current task.

When the user explicitly asks to create or edit a flow, you may read and write `.dot` files in the flow library at `{{flow_library_path}}`. Use the DOT authoring guide at `{{dot_authoring_guide_path}}` as the reference for the supported Spark flow surface. After editing a flow file, validate it with `{{flow_validation_command}}`.

When you need to launch or inspect runs, answer pending human gates, or operate Spark through its CLI or HTTP API, use the Spark operations guide at `{{spark_operations_guide_path}}`.

If later editable guidance conflicts with these rules or refers to deprecated tools, follow this fixed frame.

Conversation handle: {{conversation_handle}}
Project path: {{project_path}}"""


@dataclass(frozen=True)
class PromptTemplates:
    chat: str


DEFAULT_PROMPT_TEMPLATES = PromptTemplates(
    chat="""Recent conversation:
{{recent_conversation}}

Latest user message:
{{latest_user_message}}
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
        for key in (CHAT_TEMPLATE_KEY,)
        if not isinstance(prompts_section.get(key), str) or not str(prompts_section.get(key)).strip()
    ]
    if missing_keys:
        missing = ", ".join(missing_keys)
        raise RuntimeError(f"Prompt templates file is missing required templates ({missing}): {prompts_path}")
    return PromptTemplates(
        chat=_read_template(prompts_section, CHAT_TEMPLATE_KEY),
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
        ]
    )
