from attractor.engine.outcome import Outcome, OutcomeStatus


class TestOutcomePayload:
    def test_to_payload_includes_full_contract_fields(self):
        outcome = Outcome(
            status=OutcomeStatus.FAIL,
            preferred_label="Fix",
            suggested_next_ids=["retry_stage", "fallback_stage"],
            context_updates={
                "context.retry_count": 2,
                "context.metadata": {"source": "validator"},
            },
            notes="validation failed",
            failure_reason="lint failed",
        )

        assert outcome.to_payload() == {
            "status": "fail",
            "preferred_label": "Fix",
            "suggested_next_ids": ["retry_stage", "fallback_stage"],
            "context_updates": {
                "context.retry_count": 2,
                "context.metadata": {"source": "validator"},
            },
            "notes": "validation failed",
            "failure_reason": "lint failed",
        }

    def test_to_payload_defaults_optional_fields(self):
        outcome = Outcome(status=OutcomeStatus.SUCCESS)

        assert outcome.to_payload() == {
            "status": "success",
            "preferred_label": "",
            "suggested_next_ids": [],
            "context_updates": {},
            "notes": "",
            "failure_reason": "",
        }
