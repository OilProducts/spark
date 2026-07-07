import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { nodeTypes } from '@/features/workflow-canvas/flowCanvasShared'
import { Sidebar } from '@/features/editor/Sidebar'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, type Node, useEdgesState, useNodesState } from '@xyflow/react'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { fetchFlowListMock, deleteFlowMock } = vi.hoisted(() => ({
    fetchFlowListMock: vi.fn(async () => ['shape-test.dot']),
    deleteFlowMock: vi.fn(async () => undefined),
}))

vi.mock('@/lib/attractorClient', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@/lib/attractorClient')>()
    return {
        ...actual,
        fetchFlowListValidated: fetchFlowListMock,
        deleteFlowValidated: deleteFlowMock,
    }
})

vi.mock('@/lib/useFlowSaveScheduler', () => ({
    useFlowSaveScheduler: () => ({
        scheduleSave: vi.fn(),
        flushPendingSave: vi.fn(),
    }),
}))

const renderWithFlowProvider = (node: ReactNode) =>
    render(<ReactFlowProvider>{node}</ReactFlowProvider>)

const installDomMatrixReadOnlyStub = () => {
    class MockDOMMatrixReadOnly {
        m22: number

        constructor(transform?: string) {
            const scaleMatch = typeof transform === 'string'
                ? transform.match(/scale\(([^)]+)\)/)
                : null
            this.m22 = scaleMatch ? Number.parseFloat(scaleMatch[1]) || 1 : 1
        }
    }

    Object.defineProperty(window, 'DOMMatrixReadOnly', {
        configurable: true,
        writable: true,
        value: MockDOMMatrixReadOnly,
    })
    Object.defineProperty(globalThis, 'DOMMatrixReadOnly', {
        configurable: true,
        writable: true,
        value: MockDOMMatrixReadOnly,
    })
}

    const resetSidebarState = () => {
    useStore.setState({
        activeFlow: 'shape-test.dot',
        executionFlow: null,
        selectedNodeId: 'task',
        selectedEdgeId: null,
        diagnostics: [],
        edgeDiagnostics: {},
        graphAttrs: {},
        nodeDiagnostics: {},
        editorNodeInspectorSessionsByNodeId: {},
    })
}

const SidebarShapeHarness = ({ nodes }: { nodes: Node[] }) => {
    const [canvasNodes, , onNodesChange] = useNodesState(nodes)
    const [canvasEdges, , onEdgesChange] = useEdgesState([])

    return (
        <CanvasSessionModeProvider mode="editor">
            <div style={{ width: 900, height: 600 }}>
                <ReactFlow
                    nodes={canvasNodes}
                    edges={canvasEdges}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    nodeTypes={nodeTypes}
                    fitView
                />
            </div>
            <Sidebar />
        </CanvasSessionModeProvider>
    )
}

describe('Sidebar node shape authoring', () => {
    beforeEach(() => {
        cleanup()
        fetchFlowListMock.mockClear()
        deleteFlowMock.mockClear()
        installDomMatrixReadOnlyStub()
        resetSidebarState()
    })

    afterEach(() => {
        cleanup()
    })

    it('updates the rendered node silhouette immediately when node kind changes in the inspector', async () => {
        renderWithFlowProvider(
            <SidebarShapeHarness
                nodes={[
                    {
                        id: 'task',
                        type: 'taskNode',
                        position: { x: 0, y: 0 },
                        selected: true,
                        style: { width: 220, height: 110 },
                        data: {
                            label: 'Task',
                            kind: 'agent_task',
                            shape: 'box',
                            type: 'codergen',
                        },
                    },
                ]}
            />,
        )

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalled()
        })

        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()

        const inspectorPanel = screen.getByTestId('inspector-panel')
        const kindSelect = inspectorPanel.querySelector('select')
        expect(kindSelect).toBeTruthy()
        fireEvent.change(kindSelect as HTMLSelectElement, { target: { value: 'human_gate' } })

        await waitFor(() => {
            expect(screen.getByTestId('workflow-node-frame-hexagon')).toBeInTheDocument()
        })
    })

    it('does not expose shape/type drift warnings in the kind-based inspector', async () => {
        renderWithFlowProvider(
            <SidebarShapeHarness
                nodes={[
                    {
                        id: 'task',
                        type: 'taskNode',
                        position: { x: 0, y: 0 },
                        selected: true,
                        style: { width: 220, height: 110 },
                        data: {
                            label: 'Task',
                            kind: 'agent_task',
                            shape: 'box',
                            type: 'wait.human',
                        },
                    },
                ]}
            />,
        )

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalled()
        })

        expect(screen.queryByTestId('node-shape-type-warning')).not.toBeInTheDocument()
        expect(screen.getByText('Node Kind')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()
    })

    it('preserves advanced visibility and context drafts across remounts', async () => {
        const initialNodes: Node[] = [
            {
                id: 'task',
                type: 'taskNode',
                position: { x: 0, y: 0 },
                selected: true,
                style: { width: 220, height: 110 },
                data: {
                    label: 'Task',
                    kind: 'agent_task',
                    shape: 'box',
                    type: 'codergen',
                },
            },
        ]
        const firstRender = renderWithFlowProvider(
            <SidebarShapeHarness nodes={initialNodes} />,
        )

        await waitFor(() => {
            expect(fetchFlowListMock).toHaveBeenCalled()
        })

        fireEvent.change(screen.getByTestId('node-reads-context-editor-textarea'), {
            target: { value: 'draft.invalid' },
        })
        expect(screen.getByTestId('node-reads-context-editor-error')).toHaveTextContent(
            'Context keys must use the context.* namespace: draft.invalid',
        )

        fireEvent.click(screen.getByRole('button', { name: 'Show Advanced' }))
        expect(screen.getByRole('button', { name: 'Hide Advanced' })).toBeVisible()
        expect(screen.getByText('Max Retries')).toBeVisible()

        firstRender.unmount()
        renderWithFlowProvider(<SidebarShapeHarness nodes={initialNodes} />)

        await waitFor(() => {
            expect(screen.getByRole('button', { name: 'Hide Advanced' })).toBeVisible()
        })
        expect(screen.getByTestId('node-reads-context-editor-textarea')).toHaveValue('draft.invalid')
        expect(screen.getByTestId('node-reads-context-editor-error')).toHaveTextContent(
            'Context keys must use the context.* namespace: draft.invalid',
        )
        expect(screen.getByText('Max Retries')).toBeVisible()
    })
})
