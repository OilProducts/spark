import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'
import { DelegationsCard } from '../DelegationsCard'
import {
    answerDelegationQuestion,
    cancelDelegation,
    fetchDelegations,
    type DelegationOut,
} from '@/lib/api/environmentsApi'

vi.mock('@/lib/api/environmentsApi', () => ({
    fetchDelegations: vi.fn(),
    cancelDelegation: vi.fn(),
    answerDelegationQuestion: vi.fn(),
}))

const waitingDelegation: DelegationOut = {
    delegation_id: 'del-1',
    environment_id: 'env-b',
    project_path: '/srv/repo',
    kind: 'flow',
    request_key: 'nightly-checks',
    phase: 'accepted',
    target_id: 'run-9',
    paused: null,
    cancel_requested_at: null,
    created_at: '2026-11-23T00:00:00Z',
    updated_at: '2026-11-23T00:00:05Z',
    last_state: {
        phase: 'launched',
        status: 'waiting',
        questions: [
            {
                question_id: 'q-approve',
                prompt: 'Ship this build?',
                options: [
                    { label: 'Approve', value: 'approve' },
                    { label: 'Reject', value: 'reject' },
                ],
            },
        ],
    },
    result: null,
}

const pausedDelegation: DelegationOut = {
    ...waitingDelegation,
    delegation_id: 'del-2',
    request_key: 'paused-task',
    paused: 'The link to this environment was removed.',
    last_state: null,
}

beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(fetchDelegations).mockResolvedValue([waitingDelegation, pausedDelegation])
})

it('renders delegations, answers open questions, cancels, and filters', async () => {
    const user = userEvent.setup()
    vi.mocked(answerDelegationQuestion).mockResolvedValue(undefined)
    vi.mocked(cancelDelegation).mockResolvedValue({ ...pausedDelegation, phase: 'sealed', paused: null, result: { status: 'canceled' } })

    render(<DelegationsCard />)

    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    expect(screen.getByText('Ship this build?')).toBeInTheDocument()
    expect(screen.getByText('The link to this environment was removed.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Approve' }))
    expect(answerDelegationQuestion).toHaveBeenCalledWith('del-1', 'q-approve', { selected_value: 'approve' })

    await user.click(screen.getByRole('button', { name: 'Cancel delegation paused-task' }))
    expect(cancelDelegation).toHaveBeenCalledWith('del-2')

    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(fetchDelegations).toHaveBeenLastCalledWith({ paused: true })
})
