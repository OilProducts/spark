import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { RunQuestionsPanel } from '../RunQuestionsPanel'
import type { PendingInterviewGate } from '../../model/shared'
import { useStore } from '@/store'

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
    groupedPendingInterviewGates={[{ key: 'child', heading: 'Child run', gates: [gate] }]} now={Date.now()}
    onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
    onSubmitPendingGateAnswer={submit} pendingGateActionError={null} submittingGateIds={{}}
  />)
  expect(screen.getByText('Technical readers')).toBeInTheDocument()
  expect(screen.queryByTestId('run-pending-human-gate-keys-hint')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Developers' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'Developers', '')
  expect(screen.getByRole('textbox', { name: 'Which audience?' })).toHaveValue('Designers')
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
    groupedPendingInterviewGates={[{ key: 'root', heading: 'This run', gates: [gate] }]} now={Date.now()}
    onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
    onSubmitPendingGateAnswer={submit} pendingGateActionError={null} submittingGateIds={{}}
  />)
  expect(screen.queryByRole('button', { name: 'approve' })).toBeNull()
  // The flow's key sits in the button as a key cap and is its shortcut; the answer alone names it.
  expect(screen.getByTestId('run-pending-human-gate-option-key-approve')).toHaveTextContent('A')
  expect(screen.getByRole('button', { name: 'Approve and ship' })).toHaveAttribute('aria-keyshortcuts', 'A')
  expect(screen.queryByTestId('run-pending-human-gate-option-metadata-approve')).toBeNull()
  expect(screen.getByTestId('run-pending-human-gate-keys-hint')).toHaveTextContent('Press an answer’s key to pick it, then Enter to send it.')
  // Anywhere in Runs but a text field, a key moves focus to its answer without sending it; other keys do nothing.
  useStore.setState({ viewMode: 'runs' })
  fireEvent.keyDown(screen.getByTestId('run-pending-human-gate-note-input-gate'), { key: 'a' })
  expect(screen.getByRole('button', { name: 'Approve and ship' })).not.toHaveFocus()
  fireEvent.keyDown(document.body, { key: 'a' })
  expect(screen.getByRole('button', { name: 'Approve and ship' })).toHaveFocus()
  fireEvent.keyDown(screen.getByRole('button', { name: 'Approve and ship' }), { key: 'x' })
  expect(screen.getByRole('button', { name: 'Approve and ship' })).toHaveFocus()
  fireEvent.keyDown(screen.getByRole('button', { name: 'Approve and ship' }), { key: 's' })
  expect(screen.getByRole('button', { name: 'Send back for changes' })).toHaveFocus()
  expect(submit).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Send back for changes' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'revise', 'add a test')
  fireEvent.click(screen.getByRole('button', { name: 'Approve and ship' }))
  expect(submit).toHaveBeenLastCalledWith(gate, 'approve', 'add a test')
})

it('moves focus on from a sent answer to the next question, then to the run\'s heading', async () => {
  const user = userEvent.setup()
  const question = (id: string, prompt: string): PendingInterviewGate => ({
    eventId: `question:${id}`, questionId: id, sequence: 1, receivedAt: '2026-09-08T00:00:00Z',
    nodeId: id, stageIndex: 1, sourceScope: 'root', sourceParentNodeId: null, sourceFlowName: null,
    prompt, details: null, questionType: 'MULTIPLE_CHOICE',
    options: [{ label: 'Yes', value: 'yes', key: 'Y', description: null }],
  })
  const first = question('q1', 'Ship it?')
  const second = question('q2', 'Tag it?')
  useStore.setState((state) => ({ runsListSession: { ...state.runsListSession, selectedRunId: 'run-1' } }))
  const panel = (gates: PendingInterviewGate[], submittingGateIds: Record<string, boolean> = {}) => (
    <>
      <h1 id="run-heading-run-1" tabIndex={-1}>Run</h1>
      <RunQuestionsPanel
        confirmedQuestionIds={['q1', 'q2']}
        freeformAnswersByGateId={{}} gateNotesByGateId={{}}
        groupedPendingInterviewGates={gates.length > 0 ? [{ key: 'root', heading: 'This run', gates }] : []} now={Date.now()}
        onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
        onSubmitPendingGateAnswer={vi.fn()} pendingGateActionError={null} submittingGateIds={submittingGateIds}
      />
    </>
  )
  const { rerender } = render(panel([first, second]))

  await user.click(screen.getAllByRole('button', { name: 'Yes' })[0])
  rerender(panel([first, second], { q1: true }))
  rerender(panel([second]))
  expect(screen.getByText('Tag it?').closest('li')).toHaveFocus()

  await user.click(screen.getByRole('button', { name: 'Yes' }))
  rerender(panel([second], { q2: true }))
  rerender(panel([]))
  expect(screen.getByRole('heading', { name: 'Run' })).toHaveFocus()
})

it('answers a harness form gate with one typed reply, or declines it', async () => {
  const user = userEvent.setup()
  const gate: PendingInterviewGate = {
    eventId: 'question:form', questionId: 'form', runId: 'run', origin: 'agent_clarification',
    sequence: 1, receivedAt: '2026-09-08T00:00:00Z', nodeId: 'work', stageIndex: 1,
    sourceScope: 'root', sourceParentNodeId: null, sourceFlowName: null,
    prompt: 'Pick a size', details: null, questionType: null, options: [],
    form: {
      harness: 'acp:writer', harnessLabel: 'Writer Agent', message: 'Pick a size',
      schema: { type: 'object', properties: { size: { type: 'string', title: 'Size', enum: ['s', 'm'] }, rush: { type: 'boolean', title: 'Rush' } }, required: ['size'] },
    },
  }
  const submit = vi.fn()
  render(<RunQuestionsPanel
    confirmedQuestionIds={['form']}
    freeformAnswersByGateId={{}} gateNotesByGateId={{}}
    groupedPendingInterviewGates={[{ key: 'root', heading: 'This run', gates: [gate] }]} now={Date.now()}
    onFreeformAnswerChange={vi.fn()} onGateNoteChange={vi.fn()}
    onSubmitPendingGateAnswer={submit} pendingGateActionError={null} submittingGateIds={{}}
  />)
  expect(screen.getByRole('form', { name: 'Pick a size' })).toBeInTheDocument()
  expect(screen.getByText('Asked by Writer Agent')).toBeInTheDocument()
  expect(screen.queryByRole('textbox')).toBeNull()
  await user.click(screen.getByRole('button', { name: 'Submit' }))
  expect(submit).not.toHaveBeenCalled()
  expect(screen.getByRole('combobox', { name: 'Size' })).toHaveAttribute('aria-invalid', 'true')
  await user.selectOptions(screen.getByRole('combobox', { name: 'Size' }), 'm')
  await user.selectOptions(screen.getByRole('combobox', { name: 'Rush (optional)' }), 'No')
  await user.click(screen.getByRole('button', { name: 'Submit' }))
  expect(submit).toHaveBeenLastCalledWith(gate, '', undefined, { action: 'accept', content: { size: 'm', rush: false } })
  await user.click(screen.getByRole('button', { name: 'Decline' }))
  expect(submit).toHaveBeenLastCalledWith(gate, '', undefined, { action: 'decline' })
})
