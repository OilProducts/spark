from __future__ import annotations

from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.interviewer import Answer, Interviewer, Question, QuestionOption, QuestionType

from ..base import HandlerRuntime


class WaitHumanHandler:
    def __init__(self, interviewer: Interviewer):
        self.interviewer = interviewer

    def run(self, runtime: HandlerRuntime) -> Outcome:
        if not runtime.outgoing_edges:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="No outgoing edges for human gate")

        options = []
        for edge in runtime.outgoing_edges:
            label_attr = edge.attrs.get("label")
            label = str(label_attr.value) if label_attr else edge.target
            options.append(QuestionOption(label=label, value=label))

        question = Question(
            title=f"Human Gate: {runtime.node_id}",
            prompt=runtime.prompt or "Choose next route",
            question_type=QuestionType.SINGLE_SELECT,
            options=options,
            metadata={"node_id": runtime.node_id},
        )

        answer = self.interviewer.ask(question)
        selected = _pick_single(answer)
        if not selected:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="human skipped interaction")

        return Outcome(status=OutcomeStatus.SUCCESS, preferred_label=selected, notes="human selection applied")


def _pick_single(answer: Answer) -> str:
    if answer.selected_values:
        return answer.selected_values[0]
    return ""
