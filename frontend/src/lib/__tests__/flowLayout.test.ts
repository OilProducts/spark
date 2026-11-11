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
})
