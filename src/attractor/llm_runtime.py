from __future__ import annotations

from typing import Any, Dict, Optional, Protocol

from attractor.dsl.models import DotAttribute, DotNode


RUNTIME_LAUNCH_MODEL_KEY = "_attractor.runtime.launch_model"
RUNTIME_LAUNCH_PROVIDER_KEY = "_attractor.runtime.launch_provider"
RUNTIME_LAUNCH_PROFILE_KEY = "_attractor.runtime.launch_profile"
RUNTIME_LAUNCH_REASONING_EFFORT_KEY = "_attractor.runtime.launch_reasoning_effort"

_NON_LLM_BACKEND_SHAPES = {
    "Mdiamond",
    "Msquare",
    "hexagon",
    "diamond",
    "component",
    "parallelogram",
    "house",
}


class ContextLike(Protocol):
    def get(self, key: str, default: Any = None) -> Any:
        ...


def _normalize_optional_text(value: object) -> str:
    if value is None:
        return ""
    return str(value).strip()


def _is_default_placeholder_attr(attr: DotAttribute | None) -> bool:
    return attr is not None and attr.line <= 0


def resolve_effective_llm_model(
    node_attrs: Dict[str, DotAttribute],
    context: ContextLike,
    *,
    fallback_model: Optional[str] = None,
) -> Optional[str]:
    node_model = _normalize_optional_text(getattr(node_attrs.get("llm_model"), "value", None))
    if node_model:
        return node_model

    runtime_launch_model = _normalize_optional_text(context.get(RUNTIME_LAUNCH_MODEL_KEY, ""))
    if runtime_launch_model:
        return runtime_launch_model

    normalized_fallback = _normalize_optional_text(fallback_model)
    if normalized_fallback:
        return normalized_fallback
    return None


def resolve_effective_llm_provider(
    node_attrs: Dict[str, DotAttribute],
    context: ContextLike,
    *,
    fallback_provider: Optional[str] = None,
) -> str:
    provider_attr = node_attrs.get("llm_provider")
    node_provider = _normalize_optional_text(getattr(provider_attr, "value", None))
    if node_provider:
        return node_provider.lower()

    runtime_launch_provider = _normalize_optional_text(context.get(RUNTIME_LAUNCH_PROVIDER_KEY, ""))
    if runtime_launch_provider:
        return runtime_launch_provider.lower()

    normalized_fallback = _normalize_optional_text(fallback_provider)
    if normalized_fallback:
        return normalized_fallback.lower()
    return "codex"


def resolve_effective_llm_profile(
    node_attrs: Dict[str, DotAttribute],
    context: ContextLike,
    *,
    fallback_profile: Optional[str] = None,
) -> Optional[str]:
    profile_attr = node_attrs.get("llm_profile")
    node_profile = _normalize_optional_text(getattr(profile_attr, "value", None))
    if node_profile:
        return node_profile

    runtime_launch_profile = _normalize_optional_text(context.get(RUNTIME_LAUNCH_PROFILE_KEY, ""))
    if runtime_launch_profile:
        return runtime_launch_profile

    normalized_fallback = _normalize_optional_text(fallback_profile)
    if normalized_fallback:
        return normalized_fallback
    return None


def resolve_effective_reasoning_effort(
    node_attrs: Dict[str, DotAttribute],
    context: ContextLike,
    *,
    fallback_reasoning_effort: Optional[str] = None,
) -> Optional[str]:
    effort_attr = node_attrs.get("reasoning_effort")
    node_effort = _normalize_optional_text(getattr(effort_attr, "value", None))
    if node_effort and not _is_default_placeholder_attr(effort_attr):
        return node_effort.lower()

    runtime_effort = _normalize_optional_text(context.get(RUNTIME_LAUNCH_REASONING_EFFORT_KEY, ""))
    if runtime_effort:
        return runtime_effort.lower()

    normalized_fallback = _normalize_optional_text(fallback_reasoning_effort)
    if normalized_fallback:
        return normalized_fallback.lower()
    return None


def resolved_llm_backend_handler_type(node: DotNode) -> str:
    explicit = _normalize_optional_text(getattr(node.attrs.get("type"), "value", None))
    if explicit:
        return explicit

    shape = _normalize_optional_text(getattr(node.attrs.get("shape"), "value", None))
    if shape == "tripleoctagon":
        return "parallel.fan_in"
    if shape in _NON_LLM_BACKEND_SHAPES:
        return ""
    return "codergen"


def node_uses_llm_backend(node: DotNode) -> bool:
    return resolved_llm_backend_handler_type(node) in {"codergen", "parallel.fan_in"}
