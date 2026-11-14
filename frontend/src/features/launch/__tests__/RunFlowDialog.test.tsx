import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'

import { useStore } from '@/store'
import { RunFlowDialog, groupFlows, openRunFlowDialog } from '../components/RunFlowDialog'

const flowsByProject: Record<string, Array<{ name: string; title: string; description: string; error?: string }>> = {
    '/tmp/alpha': [
        { name: 'software-development/review-change', title: 'Review a change', description: 'Reviews the diff.' },
        { name: 'software-development/implement-change', title: 'Implement a change', description: 'Plans and builds it.' },
        { name: 'nightly', title: 'Nightly audit', description: 'Checks the build.' },
    ],
    '/tmp/beta': [{ name: 'beta-only', title: 'Beta only', description: '' }],
    // The server titles a flow that doesn't load by its file name.
    '/tmp/gamma': [{ name: 'release-check', title: 'release-check', description: '', error: 'flow.yaml: unknown field' }],
}

vi.mock('@/lib/api/flowsApi', async (original) => ({
    ...await original<typeof import('@/lib/api/flowsApi')>(),
    fetchWorkspaceFlowListValidated: vi.fn(async (projectPath: string | null) => (flowsByProject[projectPath ?? ''] ?? [])
        .map((flow) => ({ ...flow, execution_lock: null, graph_label: flow.title, graph_goal: '' }))),
}))

// alpha has a flow of its own; the rest are bundled.
vi.mock('@/features/roles/rolesModel', async (original) => ({
    ...await original<typeof import('@/features/roles/rolesModel')>(),
    fetchDefinitions: vi.fn(async (project: string) => ({
        items: project === '/tmp/alpha' ? [{ kind: 'flow', name: 'nightly', layer: 'project', path: '' }] : [],
    })),
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

it('puts the project\'s own flows first, groups the rest by folder, and matches every search word against title, description and name', () => {
    const flows = flowsByProject['/tmp/alpha'].map((flow) => ({ ...flow, execution_lock: null, graph_label: '', graph_goal: '' }))
    const names = (groups: ReturnType<typeof groupFlows>) => [
        groups.own.map((flow) => flow.name),
        groups.folders.map((group) => [group.label, group.items.map((flow) => flow.name)]),
    ]
    expect(names(groupFlows(flows, ''))).toEqual([[], [
        ['Software development', ['software-development/implement-change', 'software-development/review-change']],
        ['Other flows', ['nightly']],
    ]])
    expect(names(groupFlows(flows, '', new Set(['nightly'])))).toEqual([['nightly'], [
        ['Software development', ['software-development/implement-change', 'software-development/review-change']],
    ]])
    expect(names(groupFlows(flows, 'change diff'))).toEqual([[], [['Software development', ['software-development/review-change']]]])
    expect(names(groupFlows(flows, 'nothing-like-this'))).toEqual([[], []])
})

it('lists the chosen project\'s flows by title and opens the chosen flow\'s run form', async () => {
    const user = userEvent.setup()
    render(<RunFlowDialog />)
    act(() => openRunFlowDialog({ projectPath: '/tmp/alpha' }))

    const dialog = await screen.findByTestId('run-flow-dialog')
    expect(within(dialog).getByRole('heading', { name: 'Run a flow' })).toBeVisible()
    expect(await within(dialog).findByText('Nightly audit')).toBeVisible()
    expect(within(dialog).getAllByTestId('run-flow-option')).toHaveLength(3)
    // The project's own flows lead, then Spark's bundled ones.
    expect(await within(dialog).findByRole('heading', { name: 'alpha’s flows' })).toBeVisible()
    expect(within(dialog).getAllByTestId('run-flow-option').map((option) => option.dataset.flowName)).toEqual([
        'nightly', 'software-development/implement-change', 'software-development/review-change',
    ])
    expect(within(dialog).getByRole('heading', { name: 'Bundled with Spark' })).toBeVisible()

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

it('titles a flow that doesn\'t load as its run form does, not by its file name', async () => {
    const user = userEvent.setup()
    useStore.setState((state) => ({
        projectRegistry: { ...state.projectRegistry, '/tmp/gamma': { directoryPath: '/tmp/gamma', isFavorite: false, lastAccessedAt: null } },
    }))
    render(<RunFlowDialog />)
    act(() => openRunFlowDialog({ projectPath: '/tmp/gamma' }))
    const option = await within(await screen.findByTestId('run-flow-dialog')).findByTestId('run-flow-option')
    expect(option).toHaveTextContent(/^Release Checkrelease-checkHas errors/)
    await user.click(option)
    expect(await screen.findByTestId('launch-panel-title')).toHaveTextContent('Release Check')
})

it('opens in the project of the latest run when that is more recent than the last project used', async () => {
    useStore.setState((state) => ({
        runsListSession: {
            ...state.runsListSession,
            runs: [{ run_id: 'run-1', flow_name: 'beta-only', status: 'completed', working_directory: '/tmp/beta', project_path: '/tmp/beta', model: '', started_at: '2026-10-01T09:00:00Z' }],
        },
    }))
    render(<RunFlowDialog />)
    act(() => openRunFlowDialog())
    const dialog = await screen.findByTestId('run-flow-dialog')
    expect(within(dialog).getByTestId<HTMLSelectElement>('run-flow-project-select').value).toBe('/tmp/beta')
    expect(await within(dialog).findByText('Beta only')).toBeVisible()
})
