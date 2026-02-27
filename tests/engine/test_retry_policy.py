import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import (
    RuntimeErrorCategory,
    BackoffConfig,
    PipelineExecutor,
    RetryPolicy,
    _classify_runtime_error,
    _is_retryable_exception,
)
from attractor.engine.outcome import Outcome, OutcomeStatus


def _runner(node_id: str, prompt: str, context: Context) -> Outcome:
    return Outcome(status=OutcomeStatus.SUCCESS)


def test_retry_policy_object_uses_max_attempts_from_node_max_retries():
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box, max_retries=2]
            done [shape=Msquare]
            start -> task
            task -> done
        }
        """
    )

    executor = PipelineExecutor(graph, _runner)

    policy = executor._retry_policy_for_node("task")

    assert policy.max_attempts == 3
    assert policy.backoff.initial_delay_ms == 200
    assert policy.backoff.backoff_factor == 2.0
    assert policy.backoff.max_delay_ms == 60000
    assert policy.backoff.jitter is True


@pytest.mark.parametrize(
    ("preset", "max_attempts", "initial_delay_ms", "backoff_factor", "jitter"),
    [
        ("none", 1, 0, 1.0, False),
        ("standard", 5, 200, 2.0, True),
        ("aggressive", 5, 500, 2.0, True),
        ("linear", 3, 500, 1.0, False),
        ("patient", 3, 2000, 3.0, True),
    ],
)
def test_retry_policy_uses_named_presets(
    preset: str,
    max_attempts: int,
    initial_delay_ms: int,
    backoff_factor: float,
    jitter: bool,
):
    graph = parse_dot(
        f"""
        digraph G {{
            start [shape=Mdiamond]
            task [shape=box, retry_policy="{preset}"]
            done [shape=Msquare]
            start -> task
            task -> done
        }}
        """
    )

    executor = PipelineExecutor(graph, _runner)

    policy = executor._retry_policy_for_node("task")

    assert policy.max_attempts == max_attempts
    assert policy.backoff.initial_delay_ms == initial_delay_ms
    assert policy.backoff.backoff_factor == backoff_factor
    assert policy.backoff.jitter is jitter


def test_retry_policy_preset_overrides_max_retries_when_present():
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box, retry_policy="linear", max_retries=9]
            done [shape=Msquare]
            start -> task
            task -> done
        }
        """
    )

    executor = PipelineExecutor(graph, _runner)

    policy = executor._retry_policy_for_node("task")

    assert policy.max_attempts == 3
    assert policy.backoff.initial_delay_ms == 500
    assert policy.backoff.backoff_factor == 1.0


def test_retry_policy_should_retry_for_retry_and_retryable_fail():
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box, max_retries=1]
            done [shape=Msquare]
            start -> task
            task -> done
        }
        """
    )

    executor = PipelineExecutor(graph, _runner)

    policy = executor._retry_policy_for_node("task")

    assert policy.should_retry(Outcome(status=OutcomeStatus.RETRY)) is True
    assert policy.should_retry(Outcome(status=OutcomeStatus.FAIL, retryable=True)) is True
    assert policy.should_retry(Outcome(status=OutcomeStatus.FAIL, retryable=False)) is False
    assert policy.should_retry(Outcome(status=OutcomeStatus.SUCCESS)) is False


def test_backoff_delay_for_attempt_applies_exponential_growth_with_cap_when_no_jitter():
    config = BackoffConfig(
        initial_delay_ms=200,
        backoff_factor=2.0,
        max_delay_ms=700,
        jitter=False,
    )

    assert config.delay_for_attempt(1) == 200.0
    assert config.delay_for_attempt(2) == 400.0
    assert config.delay_for_attempt(3) == 700.0
    assert config.delay_for_attempt(4) == 700.0


def test_backoff_delay_for_attempt_applies_jitter_after_cap(monkeypatch):
    captured: dict[str, object] = {}

    def _uniform(low: float, high: float) -> float:
        captured["args"] = (low, high)
        return 1.5

    monkeypatch.setattr("attractor.engine.executor.random.uniform", _uniform)

    config = BackoffConfig(
        initial_delay_ms=500,
        backoff_factor=3.0,
        max_delay_ms=1000,
        jitter=True,
    )

    assert config.delay_for_attempt(2) == 1500.0
    assert captured["args"] == (0.5, 1.5)


def test_retry_loop_uses_computed_delay_for_retry_event():
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box, max_retries=1]
            done [shape=Msquare]
            start -> task
            task -> done
        }
        """
    )
    events: list[dict[str, object]] = []
    attempts = {"count": 0}

    def _runner(node_id: str, prompt: str, context: Context) -> Outcome:
        if node_id != "task":
            return Outcome(status=OutcomeStatus.SUCCESS)
        attempts["count"] += 1
        if attempts["count"] == 1:
            return Outcome(status=OutcomeStatus.RETRY, failure_reason="try again")
        return Outcome(status=OutcomeStatus.SUCCESS)

    executor = PipelineExecutor(graph, _runner, on_event=events.append)
    executor._retry_policy_for_node = lambda _node_id: RetryPolicy(  # type: ignore[method-assign]
        max_attempts=2,
        backoff=BackoffConfig(initial_delay_ms=250, backoff_factor=2.0, max_delay_ms=60000, jitter=False),
    )

    result = executor.run()

    assert result.status == "success"
    retry_events = [event for event in events if event["type"] == "StageRetrying"]
    assert len(retry_events) == 1
    assert retry_events[0]["attempt"] == 1
    assert retry_events[0]["delay"] == 250.0


class _StatusCodeError(RuntimeError):
    def __init__(self, code: int):
        super().__init__("")
        self.status_code = code


class _RateLimitExceededError(RuntimeError):
    pass


class _ValidationError(RuntimeError):
    pass


@pytest.mark.parametrize(
    ("error", "expected"),
    [
        (_StatusCodeError(429), True),
        (_StatusCodeError(503), True),
        (_StatusCodeError(401), False),
        (_RateLimitExceededError(""), True),
        (_ValidationError(""), False),
    ],
)
def test_default_retryability_uses_error_class_and_status_code(error: Exception, expected: bool):
    assert _is_retryable_exception(error) is expected


@pytest.mark.parametrize(
    ("error", "expected"),
    [
        (_StatusCodeError(429), RuntimeErrorCategory.RETRYABLE),
        (_StatusCodeError(401), RuntimeErrorCategory.TERMINAL),
        (_ValidationError("invalid condition syntax"), RuntimeErrorCategory.PIPELINE),
        (RuntimeError("No start node found"), RuntimeErrorCategory.PIPELINE),
        (RuntimeError("invalid credentials"), RuntimeErrorCategory.TERMINAL),
    ],
)
def test_runtime_error_classification_uses_retryable_terminal_and_pipeline_categories(
    error: Exception,
    expected: RuntimeErrorCategory,
):
    assert _classify_runtime_error(error) == expected
