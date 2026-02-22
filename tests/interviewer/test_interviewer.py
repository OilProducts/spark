import unittest

from attractor.interviewer import (
    Answer,
    AutoApproveInterviewer,
    CallbackInterviewer,
    Question,
    QuestionOption,
    QuestionType,
    QueueInterviewer,
)


class TestInterviewerImplementations(unittest.TestCase):
    def test_autoapprove_picks_first(self):
        q = Question(
            title="Pick",
            prompt="choose",
            question_type=QuestionType.SINGLE_SELECT,
            options=[QuestionOption(label="A", value="a"), QuestionOption(label="B", value="b")],
        )
        answer = AutoApproveInterviewer().ask(q)
        self.assertEqual(["a"], answer.selected_values)

    def test_callback_interviewer(self):
        interviewer = CallbackInterviewer(lambda q: Answer(selected_values=["x"]))
        answer = interviewer.ask(Question(title="T", prompt="P", question_type=QuestionType.CONFIRM))
        self.assertEqual(["x"], answer.selected_values)

    def test_queue_interviewer(self):
        interviewer = QueueInterviewer([Answer(selected_values=["first"]), Answer(text="second")])
        a1 = interviewer.ask(Question(title="1", prompt="1", question_type=QuestionType.SINGLE_SELECT))
        a2 = interviewer.ask(Question(title="2", prompt="2", question_type=QuestionType.FREE_TEXT))
        self.assertEqual(["first"], a1.selected_values)
        self.assertEqual("second", a2.text)


if __name__ == "__main__":
    unittest.main()
