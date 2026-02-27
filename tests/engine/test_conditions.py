from attractor.engine.conditions import evaluate_condition
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestConditions:
    def test_empty_condition_true(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        assert evaluate_condition("", outcome, Context())

    def test_equals_not_equals_and(self):
        outcome = Outcome(status=OutcomeStatus.FAIL, preferred_label="Fix")
        ctx = Context(values={"tests_passed": "false", "loop": {"state": "open"}})

        assert evaluate_condition("outcome=fail", outcome, ctx)
        assert evaluate_condition("outcome!=success", outcome, ctx)
        assert evaluate_condition("outcome=fail && context.tests_passed=false", outcome, ctx)
        assert not evaluate_condition("outcome=success && context.tests_passed=false", outcome, ctx)
        assert evaluate_condition("preferred_label=Fix", outcome, ctx)
        assert evaluate_condition("context.loop.state=open", outcome, ctx)

    def test_invalid_clause_false(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        assert not evaluate_condition("bad clause", outcome, Context())
        assert not evaluate_condition("unknown=foo", outcome, Context())

    def test_context_key_resolution_prefers_prefixed_then_unprefixed(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS)

        prefixed = Context(values={"context.tests_passed": "true", "tests_passed": "false"})
        assert evaluate_condition("context.tests_passed=true", outcome, prefixed)

        unprefixed = Context(values={"tests_passed": "true"})
        assert evaluate_condition("context.tests_passed=true", outcome, unprefixed)

    def test_quoted_literal_can_contain_and_delimiter(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS, preferred_label="A && B")
        assert evaluate_condition('preferred_label="A && B"', outcome, Context())

    def test_typed_literals_with_equals_and_not_equals(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS, preferred_label='Fix "Now"')
        context = Context(values={"context.retries": 2, "context.tests_passed": True})

        assert evaluate_condition(
            'context.retries=2 && context.tests_passed=true && preferred_label="Fix \\"Now\\""',
            outcome,
            context,
        )
        assert evaluate_condition(
            'context.retries!=3 && context.tests_passed!=false && preferred_label!="Fix \\"Later\\""',
            outcome,
            context,
        )
