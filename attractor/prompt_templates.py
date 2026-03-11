from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tomllib


PROMPTS_FILE_NAME = "prompts.toml"

CHAT_TEMPLATE_KEY = "chat"
EXECUTION_PLANNING_TEMPLATE_KEY = "execution_planning"


@dataclass(frozen=True)
class PromptTemplates:
    chat: str
    execution_planning: str


DEFAULT_PROMPT_TEMPLATES = PromptTemplates(
    chat="""You are the Spark Spawn assistant for the active project.
Help the user understand the project, refine user stories and specification changes, and plan implementation work.
Base your answers on the available project context and tool results.
When something is inferred rather than directly observed, say so plainly.
Keep replies concise, concrete, and practical.
When the user asks for a spec change, ground it in the current spec and repository context.
Prefer precise edits to existing spec text over speculative new features.
If the user has not specified a concrete change yet, propose at most a small grounded clarification rather than inventing a broad feature.
Once the exact change is agreed, call the draft_spec_proposal tool to capture the minimal spec edit proposal.

Project path: {{project_path}}
Recent conversation:
{{recent_conversation}}

Latest user message:
{{latest_user_message}}
""",
    execution_planning="""You are generating a tracker-ready execution card from an approved spec edit.
Respond with JSON only.
Schema: {"title": string, "summary": string, "objective": string, "work_items": [{"id": string, "title": string, "description": string, "acceptance_criteria": [string], "depends_on": [string]}]}.
Return 1-6 concrete work items.
Do not include markdown fences.

Project path: {{project_path}}
Approved spec edit proposal:
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
    except Exception:
        return DEFAULT_PROMPT_TEMPLATES
    prompts_section = payload.get("project_chat")
    if not isinstance(prompts_section, dict):
        return DEFAULT_PROMPT_TEMPLATES
    return PromptTemplates(
        chat=_read_template(prompts_section, CHAT_TEMPLATE_KEY, DEFAULT_PROMPT_TEMPLATES.chat),
        execution_planning=_read_template(
            prompts_section,
            EXECUTION_PLANNING_TEMPLATE_KEY,
            DEFAULT_PROMPT_TEMPLATES.execution_planning,
        ),
    )


def render_prompt_template(template: str, values: dict[str, str]) -> str:
    rendered = template
    for key, value in values.items():
        rendered = rendered.replace(f"{{{{{key}}}}}", value)
    return rendered


def _read_template(section: dict[object, object], key: str, default: str) -> str:
    value = section.get(key)
    if not isinstance(value, str):
        return default
    trimmed = value.strip()
    return trimmed or default


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
