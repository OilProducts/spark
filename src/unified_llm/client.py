from __future__ import annotations

import logging
from collections.abc import AsyncIterator, Mapping
from typing import Any

from .adapters.base import ProviderAdapter
from .types import StreamEvent

logger = logging.getLogger(__name__)


class Client:
    def __init__(
        self,
        providers: Mapping[str, ProviderAdapter] | None = None,
        default_provider: str | None = None,
        **config: Any,
    ) -> None:
        self.providers = dict(providers or {})
        self.default_provider = default_provider
        self.config = dict(config)

    @classmethod
    def from_env(cls, *args: Any, **kwargs: Any) -> Client:
        logger.debug("Creating placeholder Client from environment")
        return cls(*args, **kwargs)

    async def complete(self, *args: Any, **kwargs: Any) -> Any:
        logger.debug("Client.complete placeholder invoked")
        raise NotImplementedError("Client.complete is not implemented in the M1 scaffold")

    def stream(self, *args: Any, **kwargs: Any) -> AsyncIterator[StreamEvent]:
        logger.debug("Client.stream placeholder invoked")
        from .streaming import StreamEventIterator

        return StreamEventIterator(client=self, args=args, kwargs=kwargs)

    async def close(self) -> None:
        logger.debug("Client.close placeholder invoked")
        return None


__all__ = ["Client"]
