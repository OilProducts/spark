import { DialogProvider } from '@/components/app/dialog-controller'
import { Sidebar } from '@/features/editor/Sidebar'
import { FlowsPanelActions, FlowsPanelList, useFlowCatalogPanel } from '@/features/editor/FlowsPanel'
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
        activeProjectPath: '/tmp/sidebar-project',
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

// The flow list lives in the Flows panel beside the editor's inspector column.
const FlowsPanelHarness = () => {
    const catalog = useFlowCatalogPanel()
    return <><FlowsPanelActions catalog={catalog} /><FlowsPanelList catalog={catalog} /></>
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
            <FlowsPanelHarness />
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
        vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ org_path: '', bundled_path: '', org_exists: false, items: [] }) })))
    })

    afterEach(() => {
        cleanup()
        vi.restoreAllMocks()
        vi.unstubAllGlobals()
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
            .mockResolvedValueOnce(['alpha'])
            .mockResolvedValueOnce(['alpha', 'beta'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        await user.click(screen.getByRole('button', { name: 'Create flow' }))
        await user.type(screen.getByTestId('shared-dialog-input'), 'beta')
        await user.click(screen.getByTestId('shared-dialog-confirm'))

        await waitFor(() => {
            expect(saveFlowContentMock).toHaveBeenCalledWith(
                'beta',
                { flow: expect.objectContaining({
                    schema_version: '1.0',
                    id: 'beta',
                    nodes: expect.objectContaining({ start: expect.anything(), end: expect.anything() }),
                    edges: [{ from: 'start', to: 'end' }],
                }) },
            )
        })
        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBe('beta')
        })

        expect(screen.getByRole('button', { name: 'beta' })).toBeVisible()
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
            expect(deleteFlowMock).toHaveBeenCalledWith('beta.dot', '/tmp/sidebar-project')
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBeNull()
        })

        expect(screen.queryByRole('button', { name: 'beta.dot' })).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'alpha.dot' })).toBeVisible()
    })

    it('lists the active project flows and reloads them when the project changes', async () => {
        fetchFlowListMock.mockResolvedValueOnce(['alpha.dot']).mockResolvedValueOnce(['gamma.dot'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledWith('/tmp/sidebar-project')
        })
        useStore.setState({ activeProjectPath: '/tmp/other-project' })
        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenLastCalledWith('/tmp/other-project')
            expect(screen.getByRole('button', { name: 'gamma.dot' })).toBeVisible()
        })
    })
})

describe('Flows panel org badges', () => {
    afterEach(() => { vi.unstubAllGlobals() })

    it('marks only flows that differ from bundled', async () => {
        resetSidebarState()
        fetchFlowListMock.mockReset()
        fetchFlowListMock.mockResolvedValue(['plain', 'local', 'shared'])
        vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ org_path: '/o', bundled_path: '/b', org_exists: true, items: [
            { kind: 'flow', name: 'plain', layer: 'bundled', path: 'flows/plain', uncommitted: false },
            { kind: 'flow', name: 'local', layer: 'project', path: 'flows/local', uncommitted: true },
            { kind: 'flow', name: 'shared', layer: 'project_copy', path: 'flows/shared', uncommitted: false, bundled_changed: true },
        ] }) })))
        renderWithFlowProvider(<FlowsPanelHarness />)
        const badges = async (flow: string) => within(await screen.findByRole('button', { name: flow })).queryAllByTestId('org-badge').map((badge) => badge.textContent)
        await waitFor(async () => expect(await badges('local')).toEqual(['customized', 'uncommitted']))
        expect(await badges('shared')).toEqual(['project copy'])
        expect(await badges('plain')).toEqual([])
    })
})

describe('Flows panel org badges across projects', () => {
    afterEach(() => { vi.unstubAllGlobals() })

    it('never shows one project\'s badges on another project\'s flow of the same name', async () => {
        resetSidebarState()
        fetchFlowListMock.mockReset()
        fetchFlowListMock.mockResolvedValue(['shared'])
        const pendingB = createDeferred<Response>()
        vi.stubGlobal('fetch', vi.fn(async (url: string) => url.includes('project_path=%2Ftmp%2Fproject-b')
            ? pendingB.promise
            : ({ ok: true, json: async () => ({ org_path: '/a', bundled_path: '/b', org_exists: true, items: [
                { kind: 'flow', name: 'shared', layer: 'project', path: 'flows/shared', uncommitted: true },
            ] }) })))
        vi.spyOn(console, 'error').mockImplementation(() => {})
        useStore.setState({ activeProjectPath: '/tmp/project-a' })
        renderWithFlowProvider(<FlowsPanelHarness />)
        const badges = () => within(screen.getByRole('button', { name: 'shared' })).queryAllByTestId('org-badge').map((badge) => badge.textContent)
        await waitFor(() => expect(badges()).toEqual(['customized', 'uncommitted']))

        // B's org request is pending: A's badges must not carry over.
        useStore.setState({ activeProjectPath: '/tmp/project-b' })
        await waitFor(() => expect(fetchFlowListMock).toHaveBeenCalledTimes(2))
        expect(badges()).toEqual([])

        // B's org request fails: still none.
        pendingB.reject(new Error('org unavailable'))
        await waitFor(() => expect(console.error).toHaveBeenCalled())
        expect(badges()).toEqual([])
    })
})
