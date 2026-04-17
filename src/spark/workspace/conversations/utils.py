from __future__ import annotations

import re
import time
from typing import TYPE_CHECKING, Any, Optional

from spark_common.project_identity import normalize_project_path

if TYPE_CHECKING:
    from .models import ConversationTurn


def iso_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "project"


def truncate_text(value: str, limit: int) -> str:
    trimmed = value.strip()
    if len(trimmed) <= limit:
        return trimmed
    return trimmed[: max(0, limit - 1)].rstrip() + "…"


def normalize_project_path_value(value: str) -> str:
    return normalize_project_path(value)


def as_non_empty_string(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    if isinstance(value, (int, float, bool)):
        return str(value)
    return None


def derive_conversation_title(turns: list["ConversationTurn"]) -> str:
    for turn in turns:
        if turn.kind != "message" or turn.role != "user":
            continue
        title = as_non_empty_string(turn.content)
        if title:
            return truncate_text(title, 64)
    return "New thread"


def build_conversation_preview(turns: list["ConversationTurn"]) -> Optional[str]:
    for turn in reversed(turns):
        if turn.kind != "message":
            continue
        preview = as_non_empty_string(turn.content)
        if preview:
            return truncate_text(preview, 96)
    return None
