from __future__ import annotations

import asyncio
from typing import Any

import attractor.api.server as server


def preview_pipeline(flow_content: str) -> dict[str, Any]:
    return asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=flow_content)))
