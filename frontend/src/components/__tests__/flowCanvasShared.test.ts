import { buildHydratedFlowGraph, layoutWithElk } from '@/components/flowCanvasShared'
import type { PreviewResponsePayload } from '@/lib/attractorClient'
import { describe, expect, it } from 'vitest'

describe('flowCanvasShared', () => {
    it('hydrates mixed-shape graphs with shape-specific node types and dimensions', () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                graph_attrs: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'human', label: 'Human', shape: 'hexagon' },
                    { id: 'manager', label: 'Manager', shape: 'house' },
                    { id: 'custom', label: 'Custom', shape: 'ellipse' },
                ],
                edges: [],
            },
        }

        const hydrated = buildHydratedFlowGraph('shape-canvas.dot', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        expect(hydrated?.nodes).toMatchObject([
            {
                id: 'start',
                type: 'startNode',
                style: { width: 168, height: 96 },
                data: { shape: 'Mdiamond' },
            },
            {
                id: 'human',
                type: 'humanGateNode',
                style: { width: 228, height: 116 },
                data: { shape: 'hexagon' },
            },
            {
                id: 'manager',
                type: 'managerNode',
                style: { width: 236, height: 124 },
                data: { shape: 'house' },
            },
            {
                id: 'custom',
                type: 'taskNode',
                style: { width: 220, height: 110 },
                data: { shape: 'ellipse' },
            },
        ])
    })

    it('attaches ELK route geometry to hydrated edges during layout', async () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                graph_attrs: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'left', label: 'Left', shape: 'box' },
                    { id: 'right', label: 'Right', shape: 'box' },
                    { id: 'join', label: 'Join', shape: 'tripleoctagon' },
                ],
                edges: [
                    { from: 'start', to: 'left' },
                    { from: 'start', to: 'right' },
                    { from: 'left', to: 'join' },
                    { from: 'right', to: 'join' },
                ],
            },
        }

        const hydrated = buildHydratedFlowGraph('routing-canvas.dot', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        const layoutGraph = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])

        expect(layoutGraph.nodes).toHaveLength(4)
        expect(layoutGraph.edges).toHaveLength(4)
        expect(
            layoutGraph.edges.every((edge) => Array.isArray((edge.data as { layoutRoute?: unknown[] } | undefined)?.layoutRoute)),
        ).toBe(true)
    })
})
