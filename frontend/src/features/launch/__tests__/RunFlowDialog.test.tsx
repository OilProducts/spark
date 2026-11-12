import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'

import { useStore } from '@/store'
import { RunFlowDialog, groupFlows, openRunFlowDialog } from '../components/RunFlowDialog'

const flowsByProject: Record<string, Array<{ name: string; title: string; description: string }>> = {
    '/tmp/alpha': [
        { name: 'software-development/review-change', title: 'Review a change', description: 'Reviews the diff.' },
        { name: 'software-development/implement-change', title: 'Implement a change', description: 'Plans and builds it.' },
        { name: 'nightly', title: 'Nightly audit', description: 'Checks the build.' },
    ],
    '/tmp/beta': [{ name: 'beta-only', title: 'Beta only', description: '' }],
}

vi.mock('@/lib/api/flowsApi', async (original) => ({
    ...await original<typeof import('@/lib/api/flowsApi')>(),
    fetchWorkspaceFlowListValidated: vi.fn(async (projectPath: string | null) => (flowsByProject[projectPath ?? ''] ?? [])
        .map((flow) => ({ ...flow, execution_lock: null, graph_label: flow.title, graph_goal: '' }))),
}))

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    useStore.setState({
        projectRegistry: {
            '/tmp/alpha': { directoryPath: '/tmp/alpha', isFavorite: false, lastAccessedAt: null },
            '/tmp/beta': { directoryPath: '/tmp/beta', isFavorite: false, lastAccessedAt: null },
        },
    })
})

it('groups flows by folder and matches every search word against title, description and name', () => {
    const flows = flowsByProject['/tmp/alpha'].map((flow) => ({ ...flow, execution_lock: null, graph_label: '', graph_goal: '' }))
    expect(groupFlows(flows, '').map((group) => [group.label, group.items.map((flow) => flow.name)])).toEqual([
        ['Software development', ['software-development/implement-change', 'software-development/review-change']],
        ['Other flows', ['nightly']],
    ])
    expect(groupFlows(flows, 'change diff').flatMap((group) => group.items.map((flow) => flow.name))).toEqual(['software-development/review-change'])
    expect(groupFlows(flows, 'nothing-like-this')).toEqual([])
})

it('lists the chosen project\'s flows by title and opens the chosen flow\'s run form', async () => {
    const user = userEvent.setup()
    render(<RunFlowDialog />)
    act(() => openRunFlowDialog({ projectPath: '/tmp/alpha' }))

    const dialog = await screen.findByTestId('run-flow-dialog')
    expect(within(dialog).getByRole('heading', { name: 'Run a flow' })).toBeVisible()
    expect(await within(dialog).findByText('Nightly audit')).toBeVisible()
    expect(within(dialog).getAllByTestId('run-flow-option')).toHaveLength(3)

    await user.type(within(dialog).getByTestId('run-flow-search'), 'review')
    expect(within(dialog).getAllByTestId('run-flow-option').map((option) => option.dataset.flowName)).toEqual(['software-development/review-change'])

    await user.selectOptions(within(dialog).getByTestId('run-flow-project-select'), '/tmp/beta')
    expect(await within(dialog).findByText('No flow matches that search.')).toBeVisible()
    await user.clear(within(dialog).getByTestId('run-flow-search'))
    expect(within(dialog).getAllByTestId('run-flow-option').map((option) => option.dataset.flowName)).toEqual(['beta-only'])

    await user.click(within(dialog).getByTestId('run-flow-option'))
    expect(await screen.findByTestId('run-flow-back')).toBeVisible()
    await user.click(screen.getByTestId('run-flow-back'))
    expect(await screen.findByTestId('run-flow-search')).toBeVisible()
})
