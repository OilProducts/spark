from attractor.engine.context import Context
from attractor.transforms import RuntimePreambleTransform


class TestRuntimePreambleTransform:
    def test_builds_non_full_preamble_for_handoff(self):
        context = Context(
            values={
                "graph.goal": "Ship docs",
                "internal.run_id": "run-123",
                "context.release": "v1",
                "context.tests_passed": True,
                "_attractor.node_outcomes": {"start": "success"},
            }
        )
        transform = RuntimePreambleTransform()

        preamble = transform.apply("compact", context, ["start"])

        assert "carryover:compact" in preamble
        assert "goal=Ship docs" in preamble
        assert "run_id=run-123" in preamble
        assert "completed=start:success" in preamble
        assert "- context.release=v1" in preamble
        assert "- context.tests_passed=true" in preamble

    def test_includes_retry_metadata_when_retry_context_is_present(self):
        context = Context(
            values={
                "graph.goal": "Ship docs",
                "internal.run_id": "run-123",
                "context.release": "v1",
                "_attractor.runtime.retry.node_id": "blocked_exit",
                "_attractor.runtime.retry.attempt": 1,
                "_attractor.runtime.retry.max_attempts": 2,
                "_attractor.runtime.retry.failure_reason": "invalid structured status envelope",
            }
        )
        transform = RuntimePreambleTransform()

        preamble = transform.apply("summary:high", context, ["review"])

        assert "retry.node_id=blocked_exit" in preamble
        assert "retry.attempt=1" in preamble
        assert "retry.max_attempts=2" in preamble
        assert "retry.failure_reason=invalid structured status envelope" in preamble
        assert "_attractor.runtime.retry.node_id" not in preamble

    def test_returns_empty_preamble_for_full_fidelity(self):
        context = Context(values={"graph.goal": "Ship docs", "internal.run_id": "run-123"})
        transform = RuntimePreambleTransform()

        assert transform.apply("full", context, ["start"]) == ""

    def test_can_skip_sampled_context_items_for_declared_read_path(self):
        context = Context(
            values={
                "graph.goal": "Ship docs",
                "internal.run_id": "run-123",
                "context.release": "v1",
                "context.tests_passed": True,
                "_attractor.node_outcomes": {"start": "success"},
            }
        )
        transform = RuntimePreambleTransform()

        preamble = transform.apply(
            "summary:high",
            context,
            ["start"],
            include_context_items=False,
        )

        assert "carryover:summary:high" in preamble
        assert "goal=Ship docs" in preamble
        assert "run_id=run-123" in preamble
        assert "recent_stages=start:success" in preamble
        assert "context.release=v1" not in preamble
        assert "context.tests_passed=true" not in preamble
