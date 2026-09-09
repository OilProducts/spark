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
