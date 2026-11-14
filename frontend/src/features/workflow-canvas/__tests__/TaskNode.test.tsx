import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { nodeTypes } from '@/features/workflow-canvas/flowCanvasShared'
import { TaskNode } from '@/features/workflow-canvas/TaskNode'
import { getReactFlowNodeTypeForShape } from '@/lib/workflowNodeShape'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, type Edge, type Node, useEdgesState, useNodesState } from '@xyflow/react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/components/model-chooser/useModelOptions', () => ({
    useModelOptions: () => ({ projectPath: null, payload: { models: [{ provider: 'anthropic', id: 'claude-sonnet-4-6', display: 'claude-sonnet-4-6', is_default: false, supported_reasoning_efforts: ['low', 'high'] }], providers: { codex: { status: 'available', error: null } } } }),
}))

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
    {
useStore.setState({activeFlow: 'shape-test.dot',
flowMetadata: {},
graphAttrs: {},
nodeDiagnostics: {},
editorExpandChildFlowsByFlow: {}});
useStore.getState().setRunsSelectedRunId(null);
}
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
        <CanvasSessionModeProvider mode={mode} runId="run-1">
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

    it('moves to the step settings from the canvas Edit button', () => {
        renderWithFlowProvider(
            <>
                <nav data-testid="inspector-panel"><h2 tabIndex={-1}>Step settings</h2></nav>
                <SingleNodeHarness
                    node={{
                        id: 'task',
                        type: 'task',
                        position: { x: 0, y: 0 },
                        selected: true,
                        data: { label: 'Task', kind: 'agent_task', config: { kind: 'agent_task' }, shape: 'box' },
                    }}
                />
            </>,
        )

        fireEvent.click(screen.getByLabelText('Edit Task in step settings'))
        expect(screen.getByRole('heading', { name: 'Step settings' })).toHaveFocus()
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
        {
useStore.setState({});
useStore.getState().setRunsSelectedRunId('run-1');
const inspectedRunId = selectSelectedRunId(useStore.getState());
if (inspectedRunId) useStore.getState().updateRunDetailSession(inspectedRunId, {humanGate: {
                id: 'gate-1',
                runId: 'run-1',
                nodeId: 'human',
                prompt: 'Choose next step',
                options: [
                    { label: 'Continue', value: 'continue' },
                ],
            },
nodeDiagnostics: {
                human: [
                    {
                        rule_id: 'human_gate_warning',
                        severity: 'warning',
                        message: 'Human review is pending.',
                    },
                ],
            }});
}

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
        expect(screen.getByText('Needs input')).toBeInTheDocument()
        // In a run the status takes the kind word's place on the caption line, for screen readers too.
        expect(screen.getByText('Human gate')).toHaveClass('sr-only')
        expect(screen.queryByText('Human Input Required')).not.toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'Continue' })).not.toBeInTheDocument()
        expect(screen.getByTestId('node-diagnostic-badge')).toHaveTextContent('1 warning')
        expect(
            [...container.querySelectorAll('.react-flow__handle')].every((handle) =>
                handle.className.includes('pointer-events-none'),
            ),
        ).toBe(true)
    })
})
