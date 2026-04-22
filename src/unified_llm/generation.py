from __future__ import annotations

import logging
from typing import Any

from .types import _PlaceholderRecord

logger = logging.getLogger(__name__)


class GenerateResult(_PlaceholderRecord):
    pass


class StepResult(_PlaceholderRecord):
    pass


class StreamResult(_PlaceholderRecord):
    def __aiter__(self) -> StreamResult:
        return self

    async def __anext__(self) -> Any:
        logger.debug("StreamResult placeholder iterated")
        raise NotImplementedError("StreamResult is not implemented in the M1 scaffold")

    @property
    def text_stream(self) -> StreamResult:
        return self

    @property
    def partial_response(self) -> Any:
        return self.__dict__.get("partial_response")

    async def response(self) -> Any:
        logger.debug("StreamResult.response placeholder invoked")
        raise NotImplementedError("StreamResult.response is not implemented in the M1 scaffold")


async def generate(*args: Any, **kwargs: Any) -> Any:
    logger.debug("generate placeholder invoked")
    raise NotImplementedError("generate is not implemented in the M1 scaffold")


def stream(*args: Any, **kwargs: Any) -> StreamResult:
    logger.debug("stream placeholder invoked")
    return StreamResult(args=args, kwargs=kwargs)


__all__ = ["GenerateResult", "StepResult", "StreamResult", "generate", "stream"]
