from __future__ import annotations

import os
from typing import Mapping

CODEX_JSONRPC_TRACE_ENV = "SPARK_DEBUG_CODEX_JSONRPC"
CODEX_JSONRPC_TRACE_FILE_NAME = "codex-jsonrpc-trace.jsonl"
TRUTHY_ENV_VALUES = {"1", "true", "yes", "on"}


def truthy_env_value(value: object) -> bool:
    return str(value or "").strip().lower() in TRUTHY_ENV_VALUES


def codex_jsonrpc_trace_enabled(env: Mapping[str, str] | None = None) -> bool:
    source = os.environ if env is None else env
    return truthy_env_value(source.get(CODEX_JSONRPC_TRACE_ENV))
