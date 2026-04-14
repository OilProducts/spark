import { ExecutionSidebar } from '@/features/execution/ExecutionSidebar'
import { useStore } from '@/store'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { fetchFlowListMock } = vi.hoisted(() => ({
    fetchFlowListMock: vi.fn(async () => [] as string[]),
}))

vi.mock('@/lib/attractorClient', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@/lib/attractorClient')>()
    return {
        ...actual,
        fetchFlowListValidated: fetchFlowListMock,
    }
})

const createDeferred = <T,>() => {
    let resolve!: (value: T) => void
    let reject!: (reason?: unknown) => void
    const promise = new Promise<T>((nextResolve, nextReject) => {
        resolve = nextResolve
        reject = nextReject
    })
    return { promise, resolve, reject }
}

const resetExecutionSidebarState = (options?: {
    executionFlow?: string | null
    executionContinuation?: {
        sourceRunId: string
        sourceFlowName: string | null
        sourceWorkingDirectory: string
        sourceModel: string | null
        flowSourceMode: 'snapshot' | 'flow_name'
        startNodeId: string | null
    } | null
}) => {
    useStore.setState({
        activeProjectPath: '/tmp/project-one',
        executionFlow: options?.executionFlow ?? null,
        executionContinuation: options?.executionContinuation ?? null,
        humanGate: null,
    })
}

describe('Execution sidebar manual refresh', () => {
    beforeEach(() => {
        fetchFlowListMock.mockReset()
        resetExecutionSidebarState()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('refreshes the execution flow catalog manually while preserving and clearing selection as appropriate', async () => {
        const user = userEvent.setup()
        const pendingRefresh = createDeferred<string[]>()

        resetExecutionSidebarState({ executionFlow: 'alpha.dot' })
        fetchFlowListMock
            .mockResolvedValueOnce(['alpha.dot', 'beta.dot'])
            .mockImplementationOnce(() => pendingRefresh.promise)
            .mockResolvedValueOnce(['gamma.dot'])

        render(<ExecutionSidebar />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        const flowTree = await screen.findByTestId('execution-flow-tree')
        const refreshButton = screen.getByTestId('execution-flow-refresh-button')
        expect(refreshButton).toBeEnabled()
        expect(useStore.getState().executionFlow).toBe('alpha.dot')
        expect(within(flowTree).getByRole('button', { name: 'alpha.dot' })).toBeVisible()

        await user.click(refreshButton)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(refreshButton).toBeDisabled()
            expect(refreshButton).toHaveTextContent('Refreshing…')
        })

        pendingRefresh.resolve(['alpha.dot', 'gamma.dot'])

        await waitFor(() => {
            expect(within(flowTree).getByRole('button', { name: 'gamma.dot' })).toBeVisible()
        })
        expect(refreshButton).toBeEnabled()
        expect(refreshButton).toHaveTextContent('Refresh')
        expect(useStore.getState().executionFlow).toBe('alpha.dot')

        await user.click(refreshButton)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(3)
            expect(useStore.getState().executionFlow).toBeNull()
        })
        expect(within(flowTree).queryByRole('button', { name: 'alpha.dot' })).not.toBeInTheDocument()
        expect(within(flowTree).getByRole('button', { name: 'gamma.dot' })).toBeVisible()
    })

    it('clears the continuation restart node when a selected override flow disappears on refresh', async () => {
        const user = userEvent.setup()

        resetExecutionSidebarState({
            executionFlow: 'override.dot',
            executionContinuation: {
                sourceRunId: 'run-source',
                sourceFlowName: 'source.dot',
                sourceWorkingDirectory: '/tmp/project-one',
                sourceModel: 'gpt-5.4',
                flowSourceMode: 'flow_name',
                startNodeId: 'resume',
            },
        })
        fetchFlowListMock
            .mockResolvedValueOnce(['override.dot', 'other.dot'])
            .mockResolvedValueOnce(['other.dot'])

        render(<ExecutionSidebar />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        await user.click(screen.getByTestId('execution-flow-refresh-button'))

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().executionFlow).toBeNull()
        })

        expect(useStore.getState().executionContinuation?.flowSourceMode).toBe('flow_name')
        expect(useStore.getState().executionContinuation?.startNodeId).toBeNull()
        expect(screen.queryByRole('button', { name: 'override.dot' })).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'other.dot' })).toBeVisible()
    })
})
