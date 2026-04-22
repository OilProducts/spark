from __future__ import annotations

import logging

from .client import Client

logger = logging.getLogger(__name__)

_default_client: Client | None = None


def get_default_client() -> Client:
    global _default_client
    if _default_client is None:
        logger.debug("Creating placeholder default Client")
        _default_client = Client()
    return _default_client


def set_default_client(client: Client | None) -> None:
    global _default_client
    _default_client = client


__all__ = ["get_default_client", "set_default_client"]
