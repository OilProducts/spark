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
    useStore.setState((state) => ({
        ...state,
        runDiagnostics: [],
        runGraphAttrs: {},
        runDetailSessionsByRunId: {},
    }))
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
        expect(loadRunGraphPreviewMock).toHaveBeenCalledWith(run.run_id, expect.any(Object))

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
})
