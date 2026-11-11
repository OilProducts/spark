import { fireEvent, render, screen } from '@testing-library/react'
import { RunQuestionsPanel } from '../RunQuestionsPanel'
import type { PendingInterviewGate } from '../../model/shared'

it('offers suggested choices and a custom answer for the owning child run', () => {
  const gate: PendingInterviewGate = {
    eventId: 'question:q', questionId: 'q', runId: 'child', origin: 'agent_clarification',
    sequence: 1, receivedAt: '2026-09-08T00:00:00Z', nodeId: 'work', stageIndex: 1,
    sourceScope: 'child', sourceParentNodeId: 'parent', sourceFlowName: 'child-flow',
    prompt: 'Which audience?', details: null, questionType: 'FREEFORM',
    options: [{ label: 'Developers', value: 'Developers', key: null, description: 'Technical readers' }],
  }
  const submit = vi.fn()
  render(<RunQuestionsPanel
    confirmedQuestionIds={['q']}
    freeformAnswersByGateId={{ q: 'Designers' }} gateNotesByGateId={{}}
    groupedPendingInterviewGates={[{ key: 'child', heading: 'Child run', gates: [gate] }]}
    onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
    onSubmitPendingGateAnswer={submit} pendingGateActionError={null} submittingGateIds={{}}
  />)
  expect(screen.getByText('Technical readers')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Developers' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'Developers', '')
  expect(screen.getByRole('textbox', { name: 'Answer' })).toHaveValue('Designers')
  fireEvent.click(screen.getByRole('button', { name: 'Submit' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'Designers')
})

it('labels human gate choices with their display text and answers with the decision id', () => {
  const gate: PendingInterviewGate = {
    eventId: 'question:gate', questionId: 'gate', sequence: 1, receivedAt: '2026-09-08T00:00:00Z',
    nodeId: 'review_gate', stageIndex: 2, sourceScope: 'root', sourceParentNodeId: null, sourceFlowName: null,
    prompt: 'Ship this change?', details: null, questionType: 'MULTIPLE_CHOICE',
    options: [
      { label: 'Approve and ship', value: 'approve', key: 'A', description: null },
      { label: 'Send back for changes', value: 'revise', key: 'S', description: null },
    ],
  }
  const submit = vi.fn()
  render(<RunQuestionsPanel
    confirmedQuestionIds={['gate']}
    freeformAnswersByGateId={{}} gateNotesByGateId={{ gate: 'add a test' }}
    groupedPendingInterviewGates={[{ key: 'root', heading: 'This run', gates: [gate] }]}
    onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
    onSubmitPendingGateAnswer={submit} pendingGateActionError={null} submittingGateIds={{}}
  />)
  expect(screen.queryByRole('button', { name: 'approve' })).toBeNull()
  // The flow's key sits in the button as a key cap and is its shortcut; the answer alone names it.
  expect(screen.getByTestId('run-pending-human-gate-option-key-approve')).toHaveTextContent('A')
  expect(screen.getByRole('button', { name: 'Approve and ship' })).toHaveAttribute('aria-keyshortcuts', 'A')
  expect(screen.queryByTestId('run-pending-human-gate-option-metadata-approve')).toBeNull()
  // With focus on an answer, a key picks its answer; other keys do nothing.
  fireEvent.keyDown(screen.getByRole('button', { name: 'Approve and ship' }), { key: 'x' })
  expect(submit).not.toHaveBeenCalled()
  fireEvent.keyDown(screen.getByRole('button', { name: 'Approve and ship' }), { key: 's' })
  expect(submit).toHaveBeenLastCalledWith(gate, 'revise', 'add a test')
  fireEvent.click(screen.getByRole('button', { name: 'Send back for changes' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'revise', 'add a test')
  fireEvent.click(screen.getByRole('button', { name: 'Approve and ship' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'approve', 'add a test')
})
