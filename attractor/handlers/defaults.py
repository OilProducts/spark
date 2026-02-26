from __future__ import annotations

from typing import Mapping, Optional

from attractor.interviewer import AutoApproveInterviewer, Interviewer

from .base import CodergenBackend, Handler
from .builtin import (
    CodergenHandler,
    ConditionalHandler,
    ExitHandler,
    FanInHandler,
    ManagerLoopHandler,
    ParallelHandler,
    StartHandler,
    ToolHandler,
    WaitHumanHandler,
)
from .registry import HandlerRegistry


def build_default_registry(
    *,
    codergen_backend: Optional[CodergenBackend] = None,
    interviewer: Optional[Interviewer] = None,
    extra_handlers: Optional[Mapping[str, Handler]] = None,
) -> HandlerRegistry:
    if interviewer is None:
        interviewer = AutoApproveInterviewer()
    registry = HandlerRegistry()
    registry.register("start", StartHandler())
    registry.register("exit", ExitHandler())
    registry.register("codergen", CodergenHandler(codergen_backend))
    registry.register("wait.human", WaitHumanHandler(interviewer))
    registry.register("conditional", ConditionalHandler())
    registry.register("parallel", ParallelHandler())
    registry.register("parallel.fan_in", FanInHandler())
    registry.register("tool", ToolHandler())
    registry.register("stack.manager_loop", ManagerLoopHandler())
    if extra_handlers:
        for handler_type, handler in extra_handlers.items():
            registry.register(handler_type, handler)
    return registry
