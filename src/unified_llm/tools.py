from __future__ import annotations

import logging

from .types import _PlaceholderRecord

logger = logging.getLogger(__name__)


class Tool(_PlaceholderRecord):
    pass


class ToolChoice(_PlaceholderRecord):
    pass


class ToolCall(_PlaceholderRecord):
    pass


class ToolResult(_PlaceholderRecord):
    pass


__all__ = ["Tool", "ToolCall", "ToolChoice", "ToolResult"]
