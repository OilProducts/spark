from __future__ import annotations

from typing import Optional

from attractor.interviewer import AutoApproveInterviewer, Interviewer

from .base import CodergenBackend
from .builtin import (
    CodergenHandler,
    ConditionalHandler,
    ExitHandler,
    FanInHandler,
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
) -> HandlerRegistry:
    interviewer = interviewer or AutoApproveInterviewer()
    registry = HandlerRegistry()
    registry.register("start", StartHandler())
    registry.register("exit", ExitHandler())
    registry.register("codergen", CodergenHandler(codergen_backend))
    registry.register("wait.human", WaitHumanHandler(interviewer))
    registry.register("conditional", ConditionalHandler())
    registry.register("parallel", ParallelHandler())
    registry.register("parallel.fan_in", FanInHandler())
    registry.register("tool", ToolHandler())
    return registry
