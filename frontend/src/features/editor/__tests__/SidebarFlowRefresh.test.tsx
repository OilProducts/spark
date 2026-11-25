import { DialogProvider } from '@/components/app/dialog-controller'
import { Sidebar } from '@/features/editor/Sidebar'
import { FlowsPanelActions, FlowsPanelList, useFlowCatalogPanel } from '@/features/editor/FlowsPanel'
import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { switchEnvironment } from '@/test/environments'
import { ReactFlow, ReactFlowProvider, useEdgesState, useNodesState } from '@xyflow/react'
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
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
        vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ items: [] }) })))
    })

    afterEach(() => {
        cleanup()
        vi.restoreAllMocks()
        vi.unstubAllGlobals()
    })

    it('refreshes the saved flow catalog manually, keeping the open flow until another project shows', async () => {
        const user = userEvent.setup()
        const pendingRefresh = createDeferred<string[]>()

        resetSidebarState('alpha.dot')
        fetchFlowListMock
            .mockResolvedValueOnce(['alpha.dot', 'beta.dot'])
            .mockImplementationOnce(() => pendingRefresh.promise)
            .mockResolvedValueOnce(['gamma.dot'])
            .mockResolvedValueOnce(['delta.dot'])

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
        })

        pendingRefresh.resolve(['alpha.dot', 'gamma.dot'])

        await waitFor(() => {
            expect(within(flowTree).getByRole('button', { name: 'gamma.dot' })).toBeVisible()
        })
        expect(refreshButton).toBeEnabled()
        expect(refreshButton).toHaveAccessibleName('Refresh flows')
        expect(useStore.getState().activeFlow).toBe('alpha.dot')

        // Deleted elsewhere, the open flow stays open: the editor says it isn't here, or a save puts it back.
        await user.click(refreshButton)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(3)
            expect(within(flowTree).getByRole('button', { name: 'gamma.dot' })).toBeVisible()
        })
        expect(within(flowTree).queryByRole('button', { name: 'alpha.dot' })).not.toBeInTheDocument()
        expect(useStore.getState().activeFlow).toBe('alpha.dot')

        // Another project's list closes the flow left open from the one before.
        act(() => useStore.setState({ activeProjectPath: '/tmp/other-project' }))
        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(4)
            expect(useStore.getState().activeFlow).toBeNull()
        })
    })

    it('says when the flows could not load, and loads them on Try again', async () => {
        const user = userEvent.setup()
        vi.spyOn(console, 'error').mockImplementation(() => undefined)
        fetchFlowListMock
            .mockRejectedValueOnce(new Error('HTTP 503'))
            .mockResolvedValueOnce(['alpha'])

        renderWithFlowProvider(<FlowsPanelHarness />)

        const error = await screen.findByTestId('flow-list-error')
        expect(error).toHaveTextContent('Couldn’t load the flows.')
        await user.click(within(error).getByRole('button', { name: 'Try again' }))
        expect(await screen.findByRole('button', { name: 'alpha' })).toBeVisible()
        expect(screen.queryByTestId('flow-list-error')).not.toBeInTheDocument()
    })

    it('keeps create-flow refresh behavior intact', async () => {
        const user = userEvent.setup()

        fetchFlowListMock
            .mockResolvedValueOnce(['alpha'])
            .mockResolvedValueOnce(['alpha', 'beta-flow'])

        renderWithFlowProvider(<SidebarHarness />)

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(1)
        })

        await user.click(screen.getByRole('button', { name: 'New flow' }))
        const input = screen.getByTestId('shared-dialog-input')
        // A taken name can't be created over.
        await user.type(input, 'Alpha')
        expect(screen.getByTestId('shared-dialog-check')).toHaveTextContent('A flow named alpha already exists')
        expect(screen.getByTestId('shared-dialog-confirm')).toBeDisabled()
        await user.clear(input)
        // A name with spaces and capitals is saved under a portable path, titled as typed.
        await user.type(input, 'Beta Flow')
        expect(screen.getByTestId('shared-dialog-check')).toHaveTextContent('Saved as beta-flow')
        await user.click(screen.getByTestId('shared-dialog-confirm'))

        await waitFor(() => {
            expect(saveFlowContentMock).toHaveBeenCalledWith(
                'beta-flow',
                { flow: expect.objectContaining({
                    schema_version: '2',
                    id: 'beta_flow',
                    title: 'Beta Flow',
                    nodes: {
                        start: expect.objectContaining({
                            kind: 'start',
                            routes: { next: { to: 'end', description: 'Enter the first workflow step.' } },
                        }),
                        end: expect.objectContaining({
                            kind: 'exit',
                            config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } },
                        }),
                    },
                }) },
            )
        })
        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBe('beta-flow')
        })

        expect(screen.getByRole('button', { name: 'beta-flow' })).toBeVisible()
    })

    it('opens a flow created after a switch to another environment nowhere', async () => {
        const user = userEvent.setup()
        const saved = createDeferred<boolean>()
        saveFlowContentMock.mockReturnValue(saved.promise)
        fetchFlowListMock.mockResolvedValue(['alpha'])
        await switchEnvironment('env-r')
        renderWithFlowProvider(<FlowsPanelHarness />)
        await waitFor(() => expect(fetchFlowListMock).toHaveBeenCalledTimes(1))

        await user.click(screen.getByRole('button', { name: 'New flow' }))
        await user.type(screen.getByTestId('shared-dialog-input'), 'Beta')
        await user.click(screen.getByTestId('shared-dialog-confirm'))
        await waitFor(() => expect(saveFlowContentMock).toHaveBeenCalledOnce())
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        act(() => useStore.setState({ activeFlow: 'local-flow' }))
        await act(async () => saved.resolve(true))

        expect(useStore.getState().activeFlow).toBe('local-flow')
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

        await user.click(await screen.findByTitle('Delete beta.dot'))
        await user.click(await screen.findByTestId('shared-dialog-confirm'))

        await waitFor(() => {
            expect(deleteFlowMock).toHaveBeenCalledWith('beta.dot', '/tmp/sidebar-project')
            expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
            expect(useStore.getState().activeFlow).toBeNull()
        })

        expect(screen.queryByRole('button', { name: 'beta.dot' })).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'alpha.dot' })).toBeVisible()
    })

    it('leaves the flow open after a switch to another environment when a delete started before it finishes', async () => {
        const user = userEvent.setup()
        const deleted = createDeferred<undefined>()
        deleteFlowMock.mockReturnValueOnce(deleted.promise)
        fetchFlowListMock.mockResolvedValue(['alpha.dot', 'beta.dot'])
        await switchEnvironment('env-r')
        resetSidebarState('beta.dot')
        renderWithFlowProvider(<FlowsPanelHarness />)
        await waitFor(() => expect(fetchFlowListMock).toHaveBeenCalledTimes(1))

        await user.click(await screen.findByTitle('Delete beta.dot'))
        await user.click(await screen.findByTestId('shared-dialog-confirm'))
        await waitFor(() => expect(deleteFlowMock).toHaveBeenCalledOnce())
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        act(() => useStore.setState({ activeFlow: 'beta.dot' }))
        await act(async () => deleted.resolve(undefined))

        expect(useStore.getState().activeFlow).toBe('beta.dot')
    })

    it('shows a delete refused after a switch to another environment only in the workspace it was made in', async () => {
        const user = userEvent.setup()
        const deleted = createDeferred<undefined>()
        deleteFlowMock.mockReturnValueOnce(deleted.promise)
        fetchFlowListMock.mockResolvedValue(['alpha.dot', 'beta.dot'])
        await switchEnvironment('env-r')
        resetSidebarState('beta.dot')
        renderWithFlowProvider(<FlowsPanelHarness />)
        await waitFor(() => expect(fetchFlowListMock).toHaveBeenCalledTimes(1))

        await user.click(await screen.findByTitle('Delete beta.dot'))
        await user.click(await screen.findByTestId('shared-dialog-confirm'))
        await waitFor(() => expect(deleteFlowMock).toHaveBeenCalledOnce())
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        act(() => useStore.setState({ activeFlow: 'beta.dot' }))
        await act(async () => deleted.reject(new Error('The flow is running.')))

        // Nothing opens over this machine's workspace.
        expect(screen.queryByText('Flow not deleted')).not.toBeInTheDocument()
        expect(useStore.getState().activeFlow).toBe('beta.dot')

        // Back in the remote workspace, the refusal shows there.
        await switchEnvironment('env-r')
        act(() => resetStoreForEnvironmentSwitch(false))
        expect(await screen.findByTestId('shared-dialog-title')).toHaveTextContent('Flow not deleted')
        expect(screen.getByTestId('shared-dialog-description')).toHaveTextContent('The flow is running.')
    })

    it('lists a flow opened from elsewhere that was made since the list loaded', async () => {
        fetchFlowListMock.mockResolvedValueOnce(['alpha.dot']).mockResolvedValueOnce(['alpha.dot', 'lint-only.dot'])

        renderWithFlowProvider(<SidebarHarness />)

        expect(await screen.findByRole('button', { name: 'alpha.dot' })).toBeVisible()
        useStore.setState({ activeFlow: 'lint-only.dot' })
        expect(await screen.findByRole('button', { name: 'lint-only.dot' })).toBeVisible()
        expect(useStore.getState().activeFlow).toBe('lint-only.dot')
        expect(fetchFlowListMock).toHaveBeenCalledTimes(2)
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

describe('Flows panel layer badges', () => {
    afterEach(() => { vi.unstubAllGlobals() })

    it('shows a saved flow\'s new title in the list without a manual refresh', async () => {
        resetSidebarState('alpha')
        fetchFlowListMock.mockResolvedValue(['alpha'])
        let title = 'Old title'
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = String(input)
            const body = /\/workspace\/api\/flows\?/.test(url)
                ? [{ name: 'alpha', title, description: '', graph_label: '', graph_goal: '' }]
                : { items: [] }
            return { ok: true, status: 200, json: async () => body, text: async () => JSON.stringify(body) }
        }))
        renderWithFlowProvider(<FlowsPanelHarness />)
        expect(await screen.findByRole('button', { name: 'Old title (alpha)' })).toBeVisible()

        title = 'New title'
        useStore.getState().markSaveSuccess()

        expect(await screen.findByRole('button', { name: 'New title (alpha)' })).toBeVisible()
    })

    it('finds flows by every word of their title, description or name, and shows each in full on hover', async () => {
        resetSidebarState()
        fetchFlowListMock.mockResolvedValue(['nightly-audit', 'team/release'])
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const body = /\/workspace\/api\/flows\?/.test(String(input))
                ? [{ name: 'nightly-audit', title: 'Nightly audit', description: 'Checks the build every night.', graph_label: '', graph_goal: '' }]
                : { items: [] }
            return { ok: true, status: 200, json: async () => body, text: async () => JSON.stringify(body) }
        }))
        const user = userEvent.setup()
        renderWithFlowProvider(<FlowsPanelHarness />)
        const audit = await screen.findByRole('button', { name: 'Nightly audit (nightly-audit)' })
        expect(audit).toHaveAttribute('title', 'Nightly audit\nnightly-audit\nChecks the build every night.')
        const search = screen.getByRole('searchbox', { name: 'Search flows by title, name or description' })

        await user.type(search, 'BUILD night')
        expect(audit).toBeVisible()
        expect(screen.queryByRole('button', { name: 'release' })).not.toBeInTheDocument()

        await user.clear(search)
        await user.type(search, 'team/rel')
        expect(screen.getByRole('button', { name: 'release' })).toBeVisible()
        expect(screen.queryByRole('button', { name: 'Nightly audit (nightly-audit)' })).not.toBeInTheDocument()

        await user.type(search, 'x')
        expect(screen.getByTestId('flow-list-search-empty')).toHaveTextContent('No flows match the search.')
        await user.clear(search)
        expect(screen.queryByTestId('flow-list-search-empty')).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'release' })).toBeVisible()
    })

    it('marks only customized bundled flows; the project heading already marks the project\'s own', async () => {
        resetSidebarState()
        fetchFlowListMock.mockReset()
        fetchFlowListMock.mockResolvedValue(['plain', 'local', 'shared'])
        vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ items: [
            { kind: 'flow', name: 'plain', layer: 'bundled', path: 'flows/plain' },
            { kind: 'flow', name: 'local', layer: 'project', path: 'flows/local' },
            { kind: 'flow', name: 'shared', layer: 'project_copy', path: 'flows/shared' },
        ] }) })))
        renderWithFlowProvider(<FlowsPanelHarness />)
        const badges = async (flow: string) => within(await screen.findByRole('button', { name: flow })).queryAllByTestId('definition-badge').map((badge) => badge.textContent)
        await waitFor(async () => expect(await badges('shared')).toEqual(['customized']))
        expect(await badges('local')).toEqual([])
        expect(await badges('plain')).toEqual([])
    })
})

describe('Flows panel layer badges across projects', () => {
    afterEach(() => { vi.unstubAllGlobals() })

    it('never shows one project\'s badges on another project\'s flow of the same name', async () => {
        resetSidebarState()
        fetchFlowListMock.mockReset()
        fetchFlowListMock.mockResolvedValue(['shared'])
        const pendingB = createDeferred<Response>()
        vi.stubGlobal('fetch', vi.fn(async (url: string) => url.includes('project_path=%2Ftmp%2Fproject-b')
            ? pendingB.promise
            : ({ ok: true, json: async () => ({ items: [
                { kind: 'flow', name: 'shared', layer: 'project_copy', path: 'flows/shared' },
            ] }) })))
        vi.spyOn(console, 'error').mockImplementation(() => {})
        useStore.setState({ activeProjectPath: '/tmp/project-a' })
        renderWithFlowProvider(<FlowsPanelHarness />)
        const badges = () => within(screen.getByRole('button', { name: 'shared' })).queryAllByTestId('definition-badge').map((badge) => badge.textContent)
        await waitFor(() => expect(badges()).toEqual(['customized']))

        // B's definition request is pending: A's badges must not carry over.
        useStore.setState({ activeProjectPath: '/tmp/project-b' })
        await waitFor(() => expect(fetchFlowListMock).toHaveBeenCalledTimes(2))
        expect(badges()).toEqual([])

        // B's definition request fails: still none.
        pendingB.reject(new Error('definitions unavailable'))
        await waitFor(() => expect(console.error).toHaveBeenCalled())
        expect(badges()).toEqual([])
    })
})
