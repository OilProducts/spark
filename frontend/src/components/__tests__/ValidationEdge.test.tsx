import { CanvasSessionModeProvider } from '@/components/canvasSessionContext'
import { ValidationEdge } from '@/components/ValidationEdge'
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

    it('renders exact ELK route geometry when layoutRoute is present', async () => {
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
                    position: { x: 300, y: 200 },
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
                    targetX: 410,
                    targetY: 200,
                    sourcePosition: 'bottom',
                    targetPosition: 'top',
                    markerEnd: undefined,
                    style: {},
                    selected: false,
                    data: {
                        layoutRoute: [
                            { x: 110, y: 110 },
                            { x: 110, y: 160 },
                            { x: 410, y: 160 },
                            { x: 410, y: 200 },
                        ],
                    },
                } as EdgeProps}
            />,
        )

        await waitFor(() => {
            const path = document.querySelector('path.react-flow__edge-path')
            expect(path?.getAttribute('d')).toBe(
                'M 110 110 L 110 148 Q 110 160 122 160 L 398 160 Q 410 160 410 172 L 410 200',
            )
        })
    })

    it('falls back to a live orthogonal route when ELK geometry is absent', async () => {
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
            expect(path?.getAttribute('d')).toBe('M 220 55 L 265 55 Q 270 55 270 60 Q 270 65 275 65 L 320 65')
        })
    })
})
