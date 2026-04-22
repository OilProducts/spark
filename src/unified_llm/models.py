from __future__ import annotations

import logging
from dataclasses import dataclass

logger = logging.getLogger(__name__)


@dataclass(slots=True, frozen=True)
class ModelInfo:
    id: str = ""
    provider: str = ""
    display_name: str | None = None
    context_window: int | None = None
    supports_tools: bool | None = None
    supports_vision: bool | None = None
    supports_reasoning: bool | None = None


def get_model_info(model_id: str) -> ModelInfo | None:
    logger.debug("Model lookup placeholder invoked for %s", model_id)
    return None


def list_models(provider: str | None = None) -> list[ModelInfo]:
    logger.debug("Model listing placeholder invoked for provider=%s", provider)
    return []


def get_latest_model(provider: str, capability: str | None = None) -> ModelInfo | None:
    logger.debug(
        "Latest model lookup placeholder invoked for provider=%s capability=%s",
        provider,
        capability,
    )
    return None


__all__ = ["ModelInfo", "get_model_info", "get_latest_model", "list_models"]
