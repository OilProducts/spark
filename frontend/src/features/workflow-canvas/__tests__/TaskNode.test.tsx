import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { nodeTypes } from '@/features/workflow-canvas/flowCanvasShared'
import { TaskNode } from '@/features/workflow-canvas/TaskNode'
import { getReactFlowNodeTypeForShape } from '@/lib/workflowNodeShape'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, type Edge, type Node, useEdgesState, useNodesState } from '@xyflow/react'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/flowPersistence', async (importOriginal) => {
    const actual = await importOriginal<typeof import('@/lib/flowPersistence')>()
    return {
        ...actual,
        saveFlowContent: vi.fn(async () => true),
    }
})

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

const resetTaskNodeState = () => {
    useStore.setState({
        activeFlow: 'shape-test.dot',
        flowMetadata: {},
        graphAttrs: {},
        runGraphAttrs: {},
        nodeDiagnostics: {},
        runNodeDiagnostics: {},
        humanGate: null,
        selectedRunId: null,
        selectedRunRecord: null,
        selectedRunCompletedNodes: [],
        selectedRunStatusSync: 'idle',
        selectedRunStatusError: null,
        selectedRunStatusFetchedAtMs: null,
        editorExpandChildFlowsByFlow: {},
    })
}

const MixedShapeHarness = ({ nodes }: { nodes: Node[] }) => {
    const [canvasNodes, , onNodesChange] = useNodesState(nodes)
    const [canvasEdges, , onEdgesChange] = useEdgesState([])

    return (
        <CanvasSessionModeProvider mode="editor">
            <div style={{ width: 1200, height: 700 }}>
                <ReactFlow
                    nodes={canvasNodes}
                    edges={canvasEdges}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    nodeTypes={nodeTypes}
                    fitView
                />
            </div>
        </CanvasSessionModeProvider>
    )
}

const SingleNodeHarness = ({
    node,
    mode = 'editor',
    edges = [],
}: {
    node: Node
    mode?: 'editor' | 'runs'
    edges?: Edge[]
}) => {
    const [canvasNodes, , onNodesChange] = useNodesState([node])
    const [canvasEdges, , onEdgesChange] = useEdgesState(edges)

    return (
        <CanvasSessionModeProvider mode={mode}>
            <div style={{ width: 900, height: 600 }}>
                <ReactFlow
                    nodes={canvasNodes}
                    edges={canvasEdges}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    nodeTypes={{ task: TaskNode, ...nodeTypes }}
                    fitView
                />
            </div>
        </CanvasSessionModeProvider>
    )
}

describe('TaskNode', () => {
    it('falls back to the node id when no label is authored', () => {
        renderWithFlowProvider(
            <SingleNodeHarness
                mode="runs"
                node={{ id: 'implement', type: 'task', position: { x: 0, y: 0 }, data: {} }}
            />,
        )
        expect(screen.getByText('implement')).toBeInTheDocument()
        expect(screen.queryByText('Task Node')).not.toBeInTheDocument()
    })
    beforeEach(() => {
        cleanup()
        installDomMatrixReadOnlyStub()
        resetTaskNodeState()
    })

    afterEach(() => {
        cleanup()
    })

    it('renders distinct frames for the canonical Attractor node shapes', () => {
        const nodes: Node[] = [
            { id: 'start', type: getReactFlowNodeTypeForShape('Mdiamond'), position: { x: 0, y: 0 }, data: { label: 'Start', shape: 'Mdiamond' } },
            { id: 'exit', type: getReactFlowNodeTypeForShape('Msquare'), position: { x: 220, y: 0 }, data: { label: 'End', shape: 'Msquare' } },
            { id: 'task', type: getReactFlowNodeTypeForShape('box'), position: { x: 440, y: 0 }, data: { label: 'Task', shape: 'box' } },
            { id: 'human', type: getReactFlowNodeTypeForShape('hexagon'), position: { x: 0, y: 180 }, data: { label: 'Human', shape: 'hexagon' } },
            { id: 'conditional', type: getReactFlowNodeTypeForShape('diamond'), position: { x: 220, y: 180 }, data: { label: 'Branch', shape: 'diamond' } },
            { id: 'parallel', type: getReactFlowNodeTypeForShape('component'), position: { x: 440, y: 180 }, data: { label: 'Parallel', shape: 'component' } },
            { id: 'fanin', type: getReactFlowNodeTypeForShape('tripleoctagon'), position: { x: 0, y: 360 }, data: { label: 'Join', shape: 'tripleoctagon' } },
            { id: 'tool', type: getReactFlowNodeTypeForShape('parallelogram'), position: { x: 220, y: 360 }, data: { label: 'Tool', shape: 'parallelogram' } },
            { id: 'manager', type: getReactFlowNodeTypeForShape('house'), position: { x: 440, y: 360 }, data: { label: 'Manager', shape: 'house' } },
        ]

        renderWithFlowProvider(<MixedShapeHarness nodes={nodes} />)

        expect(screen.getByTestId('workflow-node-frame-Mdiamond')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-Msquare')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-hexagon')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-diamond')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-component')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-tripleoctagon')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-parallelogram')).toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-house')).toBeInTheDocument()
    })

    it('previews a shape change in the node toolbar and persists it on save', async () => {
        renderWithFlowProvider(
            <SingleNodeHarness
                node={{
                    id: 'task',
                    type: 'task',
                    position: { x: 0, y: 0 },
                    selected: true,
                    data: {
                        label: 'Task',
                        kind: 'agent_task',
                        config: { kind: 'agent_task', prompt: 'Implement the feature' },
                        shape: 'box',
                        prompt: 'Implement the feature',
                    },
                }}
            />,
        )

        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()

        fireEvent.click(screen.getByText('Edit', { selector: 'button' }))
        const toolbar = screen.getByText('Node Properties').parentElement?.parentElement
        const shapeSelect = toolbar?.querySelector('select')
        expect(shapeSelect).toBeTruthy()
        fireEvent.change(shapeSelect as HTMLSelectElement, { target: { value: 'house' } })

        expect(screen.getByTestId('workflow-node-frame-house')).toBeInTheDocument()

        fireEvent.click(screen.getByText('Save', { selector: 'button' }))

        await waitFor(() => {
            expect(screen.getByTestId('workflow-node-frame-house')).toBeInTheDocument()
        })
        expect(screen.queryByText('Node Properties')).not.toBeInTheDocument()
    })

    it('does not expose handler type drift warnings in the node toolbar', async () => {
        renderWithFlowProvider(
            <SingleNodeHarness
                node={{
                    id: 'task',
                    type: 'task',
                    position: { x: 0, y: 0 },
                    selected: true,
                    data: {
                        label: 'Task',
                        kind: 'agent_task',
                        config: { kind: 'agent_task' },
                        shape: 'box',
                    },
                }}
            />,
        )

        fireEvent.click(screen.getByText('Edit', { selector: 'button' }))

        expect(screen.queryByTestId('node-toolbar-shape-type-warning')).not.toBeInTheDocument()
        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()
    })

    it('shows and clears parallel threshold drafts in the node toolbar', async () => {
        renderWithFlowProvider(
            <SingleNodeHarness
                node={{
                    id: 'fan',
                    type: getReactFlowNodeTypeForShape('component'),
                    position: { x: 0, y: 0 },
                    selected: true,
                    data: {
                        label: 'Parallel',
                        kind: 'parallel',
                        config: { kind: 'parallel' },
                        shape: 'component',
                        join_policy: 'k_of_n',
                        join_k: '2',
                        join_quorum: '0.75',
                        max_parallel: '3',
                    },
                }}
                edges={[
                    { id: 'e1', source: 'fan', target: 'a' },
                    { id: 'e2', source: 'fan', target: 'b' },
                ]}
            />,
        )

        fireEvent.click(screen.getByText('Edit', { selector: 'button' }))

        expect(screen.getByText('K Threshold')).toBeInTheDocument()
        expect(screen.queryByText('Quorum Threshold')).not.toBeInTheDocument()

        const toolbar = screen.getByText('Node Properties').parentElement?.parentElement
        const selects = toolbar?.querySelectorAll('select')
        expect(selects?.length).toBeGreaterThanOrEqual(2)
        fireEvent.change(selects?.[1] as HTMLSelectElement, { target: { value: 'quorum' } })

        expect(screen.queryByText('K Threshold')).not.toBeInTheDocument()
        expect(screen.getByText('Quorum Threshold')).toBeInTheDocument()
        expect(screen.getByTestId('node-toolbar-attr-input-join_quorum')).toHaveValue('0.75')

        fireEvent.change(screen.getByTestId('node-toolbar-attr-input-join_quorum'), {
            target: { value: '0.6' },
        })

        fireEvent.change(selects?.[1] as HTMLSelectElement, { target: { value: 'wait_all' } })
        expect(screen.queryByText('K Threshold')).not.toBeInTheDocument()
        expect(screen.queryByText('Quorum Threshold')).not.toBeInTheDocument()

        fireEvent.change(selects?.[1] as HTMLSelectElement, { target: { value: 'k_of_n' } })
        expect(screen.getByText('K Threshold')).toBeInTheDocument()
        expect(screen.getByTestId('node-toolbar-attr-input-join_k')).toHaveValue('')
    })

    it('hides editor affordances while expanded child-flow preview mode is active', () => {
        useStore.setState((state) => ({
            ...state,
            editorExpandChildFlowsByFlow: {
                ...state.editorExpandChildFlowsByFlow,
                'shape-test.dot': true,
            },
        }))

        renderWithFlowProvider(
            <SingleNodeHarness
                node={{
                    id: 'task',
                    type: 'task',
                    position: { x: 0, y: 0 },
                    selected: true,
                    data: {
                        label: 'Task',
                        kind: 'agent_task',
                        config: { kind: 'agent_task', prompt: 'Implement the feature' },
                        shape: 'box',
                        prompt: 'Implement the feature',
                    },
                }}
            />,
        )

        expect(screen.queryByText('Edit', { selector: 'button' })).not.toBeInTheDocument()

        fireEvent.doubleClick(screen.getByText('Task'))

        expect(screen.queryByDisplayValue('Task')).not.toBeInTheDocument()
    })

    it('renders run waiting and diagnostics overlays on non-rectangular nodes', () => {
        useStore.setState({
            humanGate: {
                id: 'gate-1',
                runId: 'run-1',
                nodeId: 'human',
                prompt: 'Choose next step',
                options: [
                    { label: 'Continue', value: 'continue' },
                ],
            },
            selectedRunId: 'run-1',
            runNodeDiagnostics: {
                human: [
                    {
                        rule_id: 'human_gate_warning',
                        severity: 'warning',
                        message: 'Human review is pending.',
                    },
                ],
            },
        })

        const { container } = renderWithFlowProvider(
            <SingleNodeHarness
                mode="runs"
                node={{
                    id: 'human',
                    type: getReactFlowNodeTypeForShape('hexagon'),
                    position: { x: 0, y: 0 },
                    data: {
                        label: 'Human',
                        shape: 'hexagon',
                        status: 'waiting',
                    },
                }}
            />,
        )

        expect(screen.getByTestId('workflow-node-frame-hexagon')).toBeInTheDocument()
        expect(screen.getByText('Needs Input')).toBeInTheDocument()
        expect(screen.queryByText('Human Input Required')).not.toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'Continue' })).not.toBeInTheDocument()
        expect(screen.getByTestId('node-diagnostic-badge')).toHaveTextContent('1 Warn')
        expect(
            [...container.querySelectorAll('.react-flow__handle')].every((handle) =>
                handle.className.includes('pointer-events-none'),
            ),
        ).toBe(true)
    })
})
