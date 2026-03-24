import type { Edge } from '@xyflow/react'
import {
    buildFallbackOrthogonalRoute,
    buildPolylinePath,
    flattenElkSectionToRoute,
    getRouteMidpoint,
    stripEdgeLayoutRoutes,
} from '@/lib/edgeRouting'
import { describe, expect, it } from 'vitest'

describe('edgeRouting', () => {
    it('flattens ordered ELK edge sections into a single route', () => {
        const route = flattenElkSectionToRoute([
            {
                id: 'first',
                startPoint: { x: 10, y: 20 },
                endPoint: { x: 10, y: 60 },
                bendPoints: [{ x: 10, y: 40 }],
                outgoingSections: ['second'],
            },
            {
                id: 'second',
                incomingSections: ['first'],
                startPoint: { x: 10, y: 60 },
                bendPoints: [{ x: 80, y: 60 }],
                endPoint: { x: 80, y: 100 },
            },
        ])

        expect(route).toEqual([
            { x: 10, y: 20 },
            { x: 10, y: 60 },
            { x: 80, y: 60 },
            { x: 80, y: 100 },
        ])
    })

    it('builds downstream fallback routes from bottom to top', () => {
        const route = buildFallbackOrthogonalRoute(
            { x: 0, y: 0, width: 200, height: 100 },
            { x: 40, y: 220, width: 200, height: 100 },
        )

        expect(route[0]).toEqual({ x: 100, y: 100 })
        expect(route[route.length - 1]).toEqual({ x: 140, y: 220 })
        expect(route[1].y).toBe(route[2].y)
    })

    it('builds lateral fallback routes from side to side for near-row neighbors', () => {
        const route = buildFallbackOrthogonalRoute(
            { x: 0, y: 0, width: 200, height: 100 },
            { x: 320, y: 10, width: 200, height: 100 },
        )

        expect(route[0]).toEqual({ x: 200, y: 50 })
        expect(route[route.length - 1]).toEqual({ x: 320, y: 60 })
        expect(route[1].x).toBe(route[2].x)
    })

    it('builds loopback fallback routes off the same side for upward returns', () => {
        const route = buildFallbackOrthogonalRoute(
            { x: 200, y: 260, width: 200, height: 100 },
            { x: 180, y: 40, width: 200, height: 100 },
        )

        expect(route[0]).toEqual({ x: 200, y: 310 })
        expect(route[route.length - 1]).toEqual({ x: 180, y: 90 })
        expect(route[1].x).toBe(route[2].x)
        expect(route[1].x).toBeLessThan(route[0].x)
    })

    it('computes the midpoint along a multi-segment route by length', () => {
        const midpoint = getRouteMidpoint([
            { x: 0, y: 0 },
            { x: 0, y: 100 },
            { x: 100, y: 100 },
        ])

        expect(midpoint).toEqual({ x: 0, y: 100 })
    })

    it('rounds orthogonal corners when building the rendered path', () => {
        const path = buildPolylinePath([
            { x: 110, y: 110 },
            { x: 110, y: 160 },
            { x: 410, y: 160 },
            { x: 410, y: 200 },
        ])

        expect(path).toBe('M 110 110 L 110 148 Q 110 160 122 160 L 398 160 Q 410 160 410 172 L 410 200')
    })

    it('strips layout routes without disturbing other edge attrs', () => {
        const edges: Edge[] = [
            {
                id: 'e1',
                source: 'a',
                target: 'b',
                data: {
                    condition: 'outcome=success',
                    layoutRoute: [
                        { x: 0, y: 0 },
                        { x: 0, y: 40 },
                    ],
                },
            },
        ]

        expect(stripEdgeLayoutRoutes(edges)).toEqual([
            {
                id: 'e1',
                source: 'a',
                target: 'b',
                data: {
                    condition: 'outcome=success',
                },
            },
        ])
    })
})
