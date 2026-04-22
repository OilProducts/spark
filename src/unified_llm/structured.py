from __future__ import annotations

import logging
from typing import Any

from .generation import StreamResult

logger = logging.getLogger(__name__)


async def generate_object(*args: Any, **kwargs: Any) -> Any:
    logger.debug("generate_object placeholder invoked")
    raise NotImplementedError("generate_object is not implemented in the M1 scaffold")


def stream_object(*args: Any, **kwargs: Any) -> StreamResult:
    logger.debug("stream_object placeholder invoked")
    return StreamResult(args=args, kwargs=kwargs)


__all__ = ["generate_object", "stream_object"]
