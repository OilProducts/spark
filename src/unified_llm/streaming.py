from __future__ import annotations

import logging
from collections.abc import AsyncIterator

from .types import StreamEvent, _PlaceholderRecord

logger = logging.getLogger(__name__)


class StreamEventIterator(_PlaceholderRecord, AsyncIterator[StreamEvent]):
    def __aiter__(self) -> StreamEventIterator:
        return self

    async def __anext__(self) -> StreamEvent:
        logger.debug("StreamEventIterator placeholder iterated")
        raise NotImplementedError("StreamEventIterator is not implemented in the M1 scaffold")


__all__ = ["StreamEventIterator"]
