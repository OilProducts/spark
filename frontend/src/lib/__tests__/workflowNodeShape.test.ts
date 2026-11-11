import {
    getReactFlowNodeTypeForShape,
    getShapeNodeDimensions,
    normalizeWorkflowNodeShape,
} from '@/lib/workflowNodeShape'
import { describe, expect, it } from 'vitest'

describe('workflowNodeShape', () => {
    it('maps every canonical shape to the expected React Flow node type and dimensions', () => {
        expect(getReactFlowNodeTypeForShape('Mdiamond')).toBe('startNode')
        expect(getShapeNodeDimensions('Mdiamond')).toEqual({ width: 168, height: 92 })

        expect(getReactFlowNodeTypeForShape('Msquare')).toBe('exitNode')
        expect(getShapeNodeDimensions('Msquare')).toEqual({ width: 152, height: 76 })

        expect(getReactFlowNodeTypeForShape('box')).toBe('taskNode')
        expect(getShapeNodeDimensions('box')).toEqual({ width: 200, height: 84 })

        expect(getReactFlowNodeTypeForShape('hexagon')).toBe('humanGateNode')
        expect(getShapeNodeDimensions('hexagon')).toEqual({ width: 228, height: 92 })

        expect(getReactFlowNodeTypeForShape('diamond')).toBe('switchNode')
        expect(getShapeNodeDimensions('diamond')).toEqual({ width: 176, height: 100 })

        expect(getReactFlowNodeTypeForShape('component')).toBe('parallelNode')
        expect(getShapeNodeDimensions('component')).toEqual({ width: 212, height: 92 })

        expect(getReactFlowNodeTypeForShape('tripleoctagon')).toBe('fanInNode')
        expect(getShapeNodeDimensions('tripleoctagon')).toEqual({ width: 220, height: 100 })

        expect(getReactFlowNodeTypeForShape('parallelogram')).toBe('toolNode')
        expect(getShapeNodeDimensions('parallelogram')).toEqual({ width: 212, height: 84 })

        expect(getReactFlowNodeTypeForShape('house')).toBe('managerNode')
        expect(getShapeNodeDimensions('house')).toEqual({ width: 212, height: 104 })
    })

    it('falls back unknown shapes to the task silhouette', () => {
        expect(normalizeWorkflowNodeShape('ellipse')).toBe('box')
        expect(getReactFlowNodeTypeForShape('ellipse')).toBe('taskNode')
        expect(getShapeNodeDimensions('ellipse')).toEqual({ width: 200, height: 84 })
    })
})
