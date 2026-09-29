import { CanvasSessionModeProvider } from '@/features/workflow-canvas/canvasSessionContext'
import { ValidationEdge } from '@/features/workflow-canvas/ValidationEdge'
import { EDGE_RENDER_ROUTE_KEY } from '@/lib/flowLayout'
import { useStore } from '@/store'
import { ReactFlowProvider, type EdgeProps, type InternalNode, type Node, useStoreApi } from '@xyflow/react'
import { cleanup, render, waitFor } from '@testing-library/react'
import { useEffect, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

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

const resetState = () => {
    useStore.setState({
        edgeDiagnostics: {},
        executionEdgeDiagnostics: {},
    })
}

function readPathEndpoints(path: SVGPathElement | null) {
    const d = path?.getAttribute('d') ?? ''
    const numbers = Array.from(d.matchAll(/-?\d+(?:\.\d+)?/g), (match) => Number.parseFloat(match[0]))
    if (numbers.length < 4) {
        return null
    }

    return {
        start: { x: numbers[0], y: numbers[1] },
        end: { x: numbers[numbers.length - 2], y: numbers[numbers.length - 1] },
    }
}

function buildInternalNode(node: Node): InternalNode {
    return {
        ...node,
        width: node.width,
        height: node.height,
        measured: {
            width: node.width,
            height: node.height,
        },
        internals: {
            positionAbsolute: node.position,
            z: 0,
            userNode: node,
        },
    }
}

const EdgeHarness = ({
    sourceNode,
    targetNode,
    edgeProps,
}: {
    sourceNode: Node
    targetNode: Node
    edgeProps: EdgeProps
}) => {
    const store = useStoreApi()

    useEffect(() => {
        store.setState((state) => ({
            ...state,
            nodeLookup: new Map([
                [sourceNode.id, buildInternalNode(sourceNode)],
                [targetNode.id, buildInternalNode(targetNode)],
            ]),
        }))
    }, [sourceNode, store, targetNode])

    return (
        <CanvasSessionModeProvider mode="editor">
            <svg width="1000" height="600">
                <ValidationEdge {...edgeProps} />
            </svg>
        </CanvasSessionModeProvider>
    )
}

describe('ValidationEdge', () => {
    beforeEach(() => {
        cleanup()
        installDomMatrixReadOnlyStub()
        resetState()
    })

    afterEach(() => {
        cleanup()
    })

    it('uses live node geometry when routing hints are absent', async () => {
        renderWithFlowProvider(
            <EdgeHarness
                sourceNode={{
                    id: 'a',
                    type: 'taskNode',
                    position: { x: 0, y: 0 },
                    width: 220,
                    height: 110,
                    data: { label: 'A', shape: 'box' },
                }}
                targetNode={{
                    id: 'b',
                    type: 'taskNode',
                    position: { x: 320, y: 10 },
                    width: 220,
                    height: 110,
                    data: { label: 'B', shape: 'box' },
                }}
                edgeProps={{
                    id: 'e1',
                    source: 'a',
                    target: 'b',
                    sourceX: 110,
                    sourceY: 110,
                    targetX: 430,
                    targetY: 10,
                    sourcePosition: 'bottom',
                    targetPosition: 'top',
                    markerEnd: undefined,
                    style: {},
                    selected: false,
                } as EdgeProps}
            />,
        )

        await waitFor(() => {
            const path = document.querySelector('path.react-flow__edge-path')
            expect(readPathEndpoints(path as SVGPathElement | null)).toEqual({
                start: { x: 110, y: 110 },
                end: { x: 430, y: 10 },
            })
        })
    })

    it('renders the routed polyline geometry carried by edge render state', async () => {
        renderWithFlowProvider(
            <EdgeHarness
                sourceNode={{
                    id: 'a',
                    type: 'taskNode',
                    position: { x: 0, y: 0 },
                    width: 220,
                    height: 110,
                    data: { label: 'A', shape: 'box' },
                }}
                targetNode={{
                    id: 'b',
                    type: 'taskNode',
                    position: { x: 320, y: 10 },
                    width: 220,
                    height: 110,
                    data: { label: 'B', shape: 'box' },
                }}
                edgeProps={{
                    id: 'e1',
                    source: 'a',
                    target: 'b',
                    sourceX: 110,
                    sourceY: 110,
                    targetX: 430,
                    targetY: 10,
                    sourcePosition: 'bottom',
                    targetPosition: 'top',
                    markerEnd: undefined,
                    style: {},
                    selected: false,
                    data: {
                        [EDGE_RENDER_ROUTE_KEY]: [
                            { x: 220, y: 55 },
                            { x: 260, y: 55 },
                            { x: 260, y: 65 },
                            { x: 320, y: 65 },
                        ],
                    },
                } as EdgeProps}
            />,
        )

        await waitFor(() => {
            const path = document.querySelector('path.react-flow__edge-path')
            expect(readPathEndpoints(path as SVGPathElement | null)).toEqual({
                start: { x: 220, y: 55 },
                end: { x: 320, y: 65 },
            })
        })
    })

    it('preserves separated touch points when the routed polyline uses distinct endpoints', async () => {
        renderWithFlowProvider(
            <EdgeHarness
                sourceNode={{
                    id: 'a',
                    type: 'taskNode',
                    position: { x: 0, y: 0 },
                    width: 220,
                    height: 110,
                    data: { label: 'A', shape: 'box' },
                }}
                targetNode={{
                    id: 'b',
                    type: 'taskNode',
                    position: { x: 40, y: 220 },
                    width: 220,
                    height: 110,
                    data: { label: 'B', shape: 'box' },
                }}
                edgeProps={{
                    id: 'e1',
                    source: 'a',
                    target: 'b',
                    sourceX: 110,
                    sourceY: 110,
                    targetX: 150,
                    targetY: 220,
                    sourcePosition: 'bottom',
                    targetPosition: 'top',
                    markerEnd: undefined,
                    style: {},
                    selected: false,
                    data: {
                        [EDGE_RENDER_ROUTE_KEY]: [
                            { x: 220, y: 20 },
                            { x: 260, y: 20 },
                            { x: 260, y: 310 },
                            { x: 40, y: 310 },
                        ],
                    },
                } as EdgeProps}
            />,
        )

        await waitFor(() => {
            const path = document.querySelector('path.react-flow__edge-path')
            expect(readPathEndpoints(path as SVGPathElement | null)).toEqual({
                start: { x: 220, y: 20 },
                end: { x: 40, y: 310 },
            })
        })
    })

    it('uses the same base stroke width for normal edges as derived child edges', async () => {
        renderWithFlowProvider(
            <EdgeHarness
                sourceNode={{
                    id: 'a',
                    type: 'taskNode',
                    position: { x: 0, y: 0 },
                    width: 220,
                    height: 110,
                    data: { label: 'A', shape: 'box' },
                }}
                targetNode={{
                    id: 'b',
                    type: 'taskNode',
                    position: { x: 300, y: 0 },
                    width: 220,
                    height: 110,
                    data: { label: 'B', shape: 'box' },
                }}
                edgeProps={{
                    id: 'e1',
                    source: 'a',
                    target: 'b',
                    sourceX: 220,
                    sourceY: 55,
                    targetX: 300,
                    targetY: 55,
                    sourcePosition: 'right',
                    targetPosition: 'left',
                    markerEnd: undefined,
                    style: {},
                    selected: false,
                } as EdgeProps}
            />,
        )

        await waitFor(() => {
            const path = document.querySelector('path.react-flow__edge-path') as SVGPathElement | null
            expect(path).not.toBeNull()
            expect(path?.style.strokeWidth).toBe('1.25')
        })
    })
})
