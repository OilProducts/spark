import { DialogProvider } from '@/components/app/dialog-controller'
import { Sidebar } from '@/features/editor/Sidebar'
import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, useEdgesState, useNodesState } from '@xyflow/react'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { deleteFlowMock, fetchFlowListMock, saveFlowContentMock } = vi.hoisted(() => ({
    deleteFlowMock: vi.fn(async () => undefined),
    fetchFlowListMock: vi.fn(async () => [] as string[]),
    saveFlowContentMock: vi.fn(async () => true),
}))

vi.mock('@/lib/attractorClient', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@/lib/attractorClient')>()
    return {
        ...actual,
        deleteFlowValidated: deleteFlowMock,
        fetchFlowListValidated: fetchFlowListMock,
    }
})

vi.mock('@/lib/flowPersistence', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@/lib/flowPersistence')>()
    return {
        ...actual,
        saveFlowContent: saveFlowContentMock,
    }
})

vi.mock('@/lib/useFlowSaveScheduler', () => ({
    useFlowSaveScheduler: () => ({
        clearPendingSave: vi.fn(),
        flushPendingSave: vi.fn(),
        scheduleSave: vi.fn(),
        saveNow: vi.fn(),
    }),
}))

const createDeferred = <T,>() => {
    let resolve!: (value: T) => void
    let reject!: (reason?: unknown) => void
    const promise = new Promise<T>((nextResolve, nextReject) => {
        resolve = nextResolve
        reject = nextReject
    })
    return { promise, resolve, reject }
}

const resetSidebarState = (activeFlow: string | null = null) => {
    useStore.setState({
        activeFlow,
        executionFlow: null,
        selectedNodeId: null,
        selectedEdgeId: null,
        diagnostics: [],
        edgeDiagnostics: {},
        graphAttrs: {},
        nodeDiagnostics: {},
        editorNodeInspectorSessionsByNodeId: {},
        uiDefaults: {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        },
    })
}

const SidebarHarness = () => {
    const [nodes, , onNodesChange] = useNodesState([])
    const [edges, , onEdgesChange] = useEdgesState([])

    return (
        <CanvasSessionModeProvider mode="editor">
            <div style={{ width: 960, height: 720 }}>
                <ReactFlow
                    nodes={nodes}
                    edges={edges}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    fitView
                />
            </div>
            <Sidebar />
        </CanvasSessionModeProvider>
    )
}

const renderWithFlowProvider = (node: ReactNode) =>
    render(
        <ReactFlowProvider>
            <DialogProvider>{node}</DialogProvider>
        </ReactFlowProvider>,
    )

describe('Editor sidebar manual refresh', () => {
    beforeEach(() => {
        cleanup()
        deleteFlowMock.mockClear()
        fetchFlowListMock.mockReset()
        saveFlowContentMock.mockReset()
        saveFlowContentMock.mockResolvedValue(true)
        resetSidebarState()
    })

    afterEach(() => {
        cleanup()
        vi.restoreAllMocks()
    })

    it('refreshes the saved flow catalog manually while preserving and clearing selection as appropriate', async () => {
        const user = userEvent.setup()
        const pendingRefresh = createDeferred<string[]>()

        resetSidebarState('alpha.dot')
        fetchFlowListMock
            .mockResolvedValueOnce(['alpha.dot', 'beta.dot'])
            .mockImplementationOnce(() => pendingRefresh.promise)
            .mockResolvedValueOnce(['gamma.dot'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        const flowTree = await screen.findByTestId('editor-flow-tree')
        const refreshButton = screen.getByTestId('editor-flow-refresh-button')
        expect(refreshButton).toBeEnabled()
        expect(useStore.getState().activeFlow).toBe('alpha.dot')
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
        expect(useStore.getState().activeFlow).toBe('alpha.dot')

        await user.click(refreshButton)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(3)
            expect(useStore.getState().activeFlow).toBeNull()
        })
        expect(within(flowTree).queryByRole('button', { name: 'alpha.dot' })).not.toBeInTheDocument()
        expect(within(flowTree).getByRole('button', { name: 'gamma.dot' })).toBeVisible()
    })

    it('keeps create-flow refresh behavior intact', async () => {
        const user = userEvent.setup()

        fetchFlowListMock
            .mockResolvedValueOnce(['alpha.dot'])
            .mockResolvedValueOnce(['alpha.dot', 'beta.dot'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        await user.click(screen.getByRole('button', { name: 'Create flow' }))
        await user.type(screen.getByTestId('shared-dialog-input'), 'beta')
        await user.click(screen.getByTestId('shared-dialog-confirm'))

        await waitFor(() => {
            expect(saveFlowContentMock).toHaveBeenCalledWith(
                'beta.dot',
                expect.stringContaining('digraph beta'),
            )
        })
        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBe('beta.dot')
        })

        expect(screen.getByRole('button', { name: 'beta.dot' })).toBeVisible()
    })

    it('keeps delete-flow refresh behavior intact', async () => {
        const user = userEvent.setup()

        resetSidebarState('beta.dot')
        fetchFlowListMock
            .mockResolvedValueOnce(['alpha.dot', 'beta.dot'])
            .mockResolvedValueOnce(['alpha.dot'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        await user.click(screen.getByTitle('Delete beta.dot'))
        await user.click(await screen.findByTestId('shared-dialog-confirm'))

        await waitFor(() => {
            expect(deleteFlowMock).toHaveBeenCalledWith('beta.dot')
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBeNull()
        })

        expect(screen.queryByRole('button', { name: 'beta.dot' })).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'alpha.dot' })).toBeVisible()
    })
})
