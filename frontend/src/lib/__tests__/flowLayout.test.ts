import type { Node } from '@xyflow/react'
import { describe, expect, it } from 'vitest'

import { buildEdgeLayoutAssignments } from '@/lib/flowLayout'

const node = (id: string, x: number, y: number): Node => ({ id, position: { x, y }, data: {}, style: { width: 200, height: 84 } })

describe('flowLayout', () => {
    it('sends an edge to a lower layer out of the bottom and into the top, however far sideways', () => {
        const { assignments } = buildEdgeLayoutAssignments(
            [node('summarize', 0, 0), node('failed', 400, 132)],
            [{ id: 'e', source: 'summarize', target: 'failed' }],
        )
        expect(assignments['summarize->failed#0']).toMatchObject({ sourceSide: 'bottom', targetSide: 'top' })
    })

    it('joins nodes in one layer side to side', () => {
        const { assignments } = buildEdgeLayoutAssignments(
            [node('a', 0, 0), node('b', 300, 10)],
            [{ id: 'e', source: 'a', target: 'b' }],
        )
        expect(assignments['a->b#0']).toMatchObject({ sourceSide: 'right', targetSide: 'left' })
    })

    it('orders a side\'s ports by how their routes leave it, so no route\'s turn crosses another\'s', () => {
        const nodes = [node('a', 0, 0), node('left', -100, 300), node('right', 150, 300)]
        const edges = [{ id: 'l', source: 'a', target: 'left' }, { id: 'r', source: 'a', target: 'right' }]
        const byTargets = buildEdgeLayoutAssignments(nodes, edges).assignments
        expect([byTargets['a->left#0'].sourceSlot, byTargets['a->right#0'].sourceSlot]).toEqual([0, 1])

        // The route to `right` swings round something to the left of `a`; the one to `left` heads right.
        const byRoutes = buildEdgeLayoutAssignments(nodes, edges, null, undefined, {
            'a->left#0': [{ x: 66, y: 84 }, { x: 66, y: 108 }, { x: 260, y: 108 }, { x: 260, y: 300 }],
            'a->right#0': [{ x: 133, y: 84 }, { x: 133, y: 108 }, { x: -40, y: 108 }, { x: -40, y: 300 }],
        }).assignments
        expect([byRoutes['a->left#0'].sourceSlot, byRoutes['a->right#0'].sourceSlot]).toEqual([1, 0])

        // Both turn right: the one that runs on further before turning takes the left port, so the
        // other's early turn does not cut across it.
        const bothRight = buildEdgeLayoutAssignments(nodes, edges, null, undefined, {
            'a->left#0': [{ x: 66, y: 84 }, { x: 66, y: 400 }, { x: 300, y: 400 }, { x: 300, y: 420 }],
            'a->right#0': [{ x: 133, y: 84 }, { x: 133, y: 108 }, { x: 250, y: 108 }, { x: 250, y: 300 }],
        }).assignments
        expect([bothRight['a->left#0'].sourceSlot, bothRight['a->right#0'].sourceSlot]).toEqual([0, 1])
        const swapped = buildEdgeLayoutAssignments(nodes, edges, null, undefined, {
            'a->left#0': [{ x: 133, y: 84 }, { x: 133, y: 400 }, { x: 300, y: 400 }, { x: 300, y: 420 }],
            'a->right#0': [{ x: 66, y: 84 }, { x: 66, y: 108 }, { x: 250, y: 108 }, { x: 250, y: 300 }],
        }).assignments
        expect([swapped['a->left#0'].sourceSlot, swapped['a->right#0'].sourceSlot]).toEqual([0, 1])
    })
})
