import {
    buildRoundedPolylinePath,
    getPortAnchorPoint,
    getRouteMidpoint,
    routeFixedNodeGraph,
    routeIntersectsRect,
    separateOverlappingSegments,
} from '@/lib/edgeRouting'
import { describe, expect, it } from 'vitest'

describe('edgeRouting', () => {
    it('anchors ports by side and slot count', () => {
        expect(getPortAnchorPoint(
            { x: 0, y: 0, width: 220, height: 110 },
            { side: 'right', slot: 0, slotCount: 4 },
        )).toEqual({ x: 220, y: 22 })

        expect(getPortAnchorPoint(
            { x: 320, y: 220, width: 220, height: 110 },
            { side: 'left', slot: 3, slotCount: 4 },
        )).toEqual({ x: 320, y: 308 })
    })

    it('builds rounded polyline paths', () => {
        expect(buildRoundedPolylinePath([
            { x: 110, y: 110 },
            { x: 110, y: 160 },
            { x: 410, y: 160 },
            { x: 410, y: 200 },
        ])).toBe('M 110 110 L 110 148 Q 110 160 122 160 L 398 160 Q 410 160 410 172 L 410 200')
    })

    it('finds the midpoint along a routed polyline', () => {
        expect(getRouteMidpoint([
            { x: 0, y: 0 },
            { x: 0, y: 100 },
            { x: 100, y: 100 },
        ])).toEqual({ x: 0, y: 100 })
    })

    it('routes around fixed node obstacles', () => {
        const routes = routeFixedNodeGraph({
            nodes: [
                {
                    id: 'source',
                    rect: { x: 0, y: 0, width: 220, height: 110 },
                },
                {
                    id: 'target',
                    rect: { x: 440, y: 0, width: 220, height: 110 },
                },
                {
                    id: 'obstacle',
                    rect: { x: 270, y: 20, width: 110, height: 120 },
                },
            ],
            edges: [
                {
                    id: 'source->target#0',
                    source: 'source',
                    target: 'target',
                    sourcePort: { side: 'right', slot: 0, slotCount: 1 },
                    targetPort: { side: 'left', slot: 0, slotCount: 1 },
                },
            ],
        }).routes

        const route = routes['source->target#0']
        expect(route).toBeDefined()
        expect(routeIntersectsRect(route, { x: 270, y: 20, width: 110, height: 120 }, 0)).toBe(false)
    })

    // simple-linear's last layer: Summarize above, Done and Failed side by side, 40px apart.
    const done = { x: 12, y: 630, width: 168, height: 96 }
    const summarize = { x: 22, y: 472, width: 220, height: 110 }
    const failed = { x: 220, y: 630, width: 168, height: 96 }

    it('keeps a port lead in the gap before a close neighbour instead of crossing it', () => {
        const route = routeFixedNodeGraph({
            nodes: [{ id: 'summarize', rect: summarize }, { id: 'done', rect: done }, { id: 'failed', rect: failed }],
            edges: [{
                id: 'summarize->failed#0',
                source: 'summarize',
                target: 'failed',
                sourcePort: { side: 'right', slot: 0, slotCount: 1 },
                targetPort: { side: 'left', slot: 0, slotCount: 1 },
            }],
        }).routes['summarize->failed#0']

        expect(routeIntersectsRect(route, done, 0)).toBe(false)
        expect(route.at(-1)).toEqual({ x: 220, y: 678 })
    })

    it('spreads edges that share a corridor line apart, leaving their ports in place', () => {
        const separated = separateOverlappingSegments({
            a: [{ x: 0, y: 0 }, { x: 0, y: 50 }, { x: 200, y: 50 }, { x: 200, y: 100 }],
            b: [{ x: 20, y: 0 }, { x: 20, y: 50 }, { x: 300, y: 50 }, { x: 300, y: 100 }],
            c: [{ x: 400, y: 0 }, { x: 400, y: 50 }, { x: 500, y: 50 }, { x: 500, y: 100 }],
        })

        expect(separated.a[1].y).not.toBe(separated.b[1].y)
        expect(Math.abs(separated.a[1].y - 50)).toBeLessThanOrEqual(10)
        expect(separated.a[0]).toEqual({ x: 0, y: 0 })
        expect(separated.b.at(-1)).toEqual({ x: 300, y: 100 })
        // An edge alone on its stretch of the line stays on it.
        expect(separated.c[1].y).toBe(50)
    })
})

