import { CanvasSessionModeProvider } from '@/components/canvasSessionContext'
import { nodeTypes } from '@/components/flowCanvasShared'
import { TaskNode } from '@/components/TaskNode'
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
        executionFlow: 'shape-test.dot',
        graphAttrs: {},
        executionGraphAttrs: {},
        nodeDiagnostics: {},
        executionNodeDiagnostics: {},
        humanGate: null,
        selectedRunId: null,
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
    mode?: 'editor' | 'execution'
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
                        shape: 'box',
                        type: 'codergen',
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

    it('shows shape/type drift warnings in the node toolbar', async () => {
        renderWithFlowProvider(
            <SingleNodeHarness
                node={{
                    id: 'task',
                    type: 'task',
                    position: { x: 0, y: 0 },
                    selected: true,
                    data: {
                        label: 'Task',
                        shape: 'box',
                        type: 'wait.human',
                    },
                }}
            />,
        )

        fireEvent.click(screen.getByText('Edit', { selector: 'button' }))

        expect(screen.getByTestId('node-toolbar-shape-type-warning')).toHaveTextContent(
            'Shape box normally maps to codergen',
        )
        expect(screen.getByTestId('workflow-node-frame-box')).toBeInTheDocument()
    })

    it('renders execution waiting and diagnostics overlays on non-rectangular nodes', () => {
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
            executionNodeDiagnostics: {
                human: [
                    {
                        rule_id: 'human_gate_warning',
                        severity: 'warning',
                        message: 'Human review is pending.',
                    },
                ],
            },
        })

        renderWithFlowProvider(
            <SingleNodeHarness
                mode="execution"
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
        expect(screen.getByTestId('node-diagnostic-badge')).toHaveTextContent('1 Warn')
    })
})
