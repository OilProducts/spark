import { RunGraphCard } from '@/features/runs/components/RunGraphCard'
import { useStore } from '@/store'
import { act, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { loadRunGraphPreviewMock } = vi.hoisted(() => ({
    loadRunGraphPreviewMock: vi.fn(),
}))

vi.mock('@/features/runs/services/runGraphTransport', () => ({
    loadRunGraphPreview: loadRunGraphPreviewMock,
}))

const makeRun = (runId = 'run-graph') => ({
    run_id: runId,
    flow_name: 'review.dot',
    status: 'completed' as const,
    outcome: 'success' as const,
    outcome_reason_code: null,
    outcome_reason_message: null,
    working_directory: '/tmp/project-one/workdir',
    project_path: '/tmp/project-one',
    git_branch: 'main',
    git_commit: 'abcdef0',
    spec_id: null,
    plan_id: null,
    model: 'gpt-5.4',
    started_at: '2026-03-22T00:00:00Z',
    ended_at: '2026-03-22T00:05:00Z',
    last_error: null,
    token_usage: 89,
    current_node: 'done',
    continued_from_run_id: null,
    continued_from_node: null,
    continued_from_flow_mode: null,
    continued_from_flow_name: null,
})

const resetRunGraphState = () => {
    useStore.setState(useStore.getInitialState(), true)
}

describe('RunGraphCard', () => {
    beforeEach(() => {
        resetRunGraphState()
        loadRunGraphPreviewMock.mockReset()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('shows restoring state before the first authoritative graph load instead of an empty placeholder', async () => {
        const run = makeRun()
        useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
        let resolvePreview!: (value: unknown) => void
        loadRunGraphPreviewMock.mockImplementation(() => new Promise((resolve) => {
            resolvePreview = resolve
        }))

        act(() => {
            useStore.getState().updateRunDetailSession(run.run_id, {
                isGraphCollapsed: false,
            })
        })

        render(<RunGraphCard run={run} />)

        await waitFor(() => {
            expect(screen.getByTestId('run-graph-loading')).toBeVisible()
        })
        expect(screen.queryByText('No run graph preview is available for this run.')).not.toBeInTheDocument()
        expect(loadRunGraphPreviewMock).toHaveBeenCalledWith(
            run.run_id,
            expect.any(Object),
            { expandChildren: false },
        )

        await act(async () => {
            resolvePreview({
                status: 'ok',
                graph: {
                    graph_attrs: {},
                    nodes: [],
                    edges: [],
                },
                diagnostics: [],
                errors: [],
            })
        })

        await waitFor(() => {
            expect(screen.getByText('No run graph preview is available for this run.')).toBeVisible()
        })
    })

    it('reloads the run graph with expanded child previews when the toggle is enabled', async () => {
        const run = makeRun('run-expanded')
        useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
        loadRunGraphPreviewMock.mockResolvedValue({
            status: 'ok',
            graph: {
                graph_attrs: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'manager', label: 'Manager', shape: 'house' },
                ],
                edges: [
                    { from: 'start', to: 'manager' },
                ],
            },
            diagnostics: [],
            errors: [],
        })

        act(() => {
            useStore.getState().updateRunDetailSession(run.run_id, {
                isGraphCollapsed: false,
            })
        })

        render(<RunGraphCard run={run} />)

        await screen.findByTestId('run-graph-panel')
        await act(async () => {
            screen.getByRole('button', { name: 'Child flows' }).click()
        })

        await waitFor(() => {
            expect(loadRunGraphPreviewMock).toHaveBeenLastCalledWith(
                run.run_id,
                expect.any(Object),
                { expandChildren: true },
            )
        })
        expect(useStore.getState().runDetailSessionsByRunId[run.run_id]?.expandChildFlows).toBe(true)
    })

    it('insets canvas controls inside the default-height viewport', async () => {
        const run = makeRun('run-controls')
        useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
        loadRunGraphPreviewMock.mockResolvedValue({
            status: 'ok',
            graph: {
                graph_attrs: {},
                nodes: [{ id: 'start', label: 'Start', shape: 'Mdiamond' }],
                edges: [],
            },
            diagnostics: [],
            errors: [],
        })

        act(() => {
            useStore.getState().updateRunDetailSession(run.run_id, {
                isGraphCollapsed: false,
            })
        })

        render(
            <RunGraphCard
                run={run}
                nodeStatusesById={{}}
                selectedNodeId={null}
                onSelectNode={vi.fn()}
            />,
        )

        await screen.findByTestId('rf__node-start')
        const controls = screen.getByTestId('run-graph-canvas').querySelector('.react-flow__controls')
        expect(controls).toHaveStyle({ bottom: '12px', left: '12px' })
    })
})


it('rejects a delayed graph failure after its session is removed and recreated', async () => {
    useStore.setState(useStore.getInitialState(), true)
    const run = makeRun('recreated')
    useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
    let rejectPreview!: (error: Error) => void
    loadRunGraphPreviewMock.mockImplementation(() => new Promise((_, reject) => { rejectPreview = reject }))
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<RunGraphCard run={run} nodeStatusesById={{}} selectedNodeId={null} onSelectNode={() => {}} />)
    act(() => {
        useStore.getState().clearRunDetailSession(run.run_id)
        useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
    })
    await act(async () => rejectPreview(new Error('obsolete graph failure')))
    expect(useStore.getState().runDetailSessionsByRunId[run.run_id].graphError).toBeNull()
    error.mockRestore()
})

it('retains a cached graph on refresh failure and hides it when the child variant changes', async () => {
    useStore.setState(useStore.getInitialState(), true)
    const run = makeRun('cached-variant')
    useStore.getState().setRunsSelectedRunIdForScope('all', run.run_id)
    useStore.getState().updateRunDetailSession(run.run_id, {
        graphExpanded: false, graphStatus: 'ready',
        graphNodes: [{ id: 'cached', type: 'default', position: { x: 0, y: 0 }, data: { label: 'Cached graph node' } }],
    })
    let rejectPreview!: (error: Error) => void
    loadRunGraphPreviewMock.mockImplementation(() => new Promise((_, reject) => { rejectPreview = reject }))
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<RunGraphCard run={run} nodeStatusesById={{}} selectedNodeId={null} onSelectNode={() => {}} />)
    expect(screen.getByText('Cached graph node')).toBeInTheDocument()
    await act(async () => rejectPreview(new Error('refresh failure')))
    expect(screen.getByText('Cached graph node')).toBeInTheDocument()
    const calls = loadRunGraphPreviewMock.mock.calls.length
    act(() => screen.getByTestId('run-graph-refresh-button').click())
    expect(loadRunGraphPreviewMock.mock.calls.length).toBe(calls + 1)
    act(() => useStore.getState().updateRunDetailSession(run.run_id, { expandChildFlows: true }))
    expect(screen.queryByText('Cached graph node')).not.toBeInTheDocument()
    expect(useStore.getState().runDetailSessionsByRunId[run.run_id].graphNodes).toHaveLength(1)
    error.mockRestore()
})
