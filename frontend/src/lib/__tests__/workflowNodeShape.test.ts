import {
    getReactFlowNodeTypeForShape,
    getShapeNodeDimensions,
    getShapeTypeMismatchWarning,
    normalizeWorkflowNodeShape,
} from '@/lib/workflowNodeShape'
import { describe, expect, it } from 'vitest'

describe('workflowNodeShape', () => {
    it('maps every canonical shape to the expected React Flow node type and dimensions', () => {
        expect(getReactFlowNodeTypeForShape('Mdiamond')).toBe('startNode')
        expect(getShapeNodeDimensions('Mdiamond')).toEqual({ width: 168, height: 96 })

        expect(getReactFlowNodeTypeForShape('Msquare')).toBe('exitNode')
        expect(getShapeNodeDimensions('Msquare')).toEqual({ width: 168, height: 96 })

        expect(getReactFlowNodeTypeForShape('box')).toBe('taskNode')
        expect(getShapeNodeDimensions('box')).toEqual({ width: 220, height: 110 })

        expect(getReactFlowNodeTypeForShape('hexagon')).toBe('humanGateNode')
        expect(getShapeNodeDimensions('hexagon')).toEqual({ width: 228, height: 116 })

        expect(getReactFlowNodeTypeForShape('diamond')).toBe('conditionalNode')
        expect(getShapeNodeDimensions('diamond')).toEqual({ width: 176, height: 104 })

        expect(getReactFlowNodeTypeForShape('component')).toBe('parallelNode')
        expect(getShapeNodeDimensions('component')).toEqual({ width: 236, height: 116 })

        expect(getReactFlowNodeTypeForShape('tripleoctagon')).toBe('fanInNode')
        expect(getShapeNodeDimensions('tripleoctagon')).toEqual({ width: 236, height: 116 })

        expect(getReactFlowNodeTypeForShape('parallelogram')).toBe('toolNode')
        expect(getShapeNodeDimensions('parallelogram')).toEqual({ width: 228, height: 116 })

        expect(getReactFlowNodeTypeForShape('house')).toBe('managerNode')
        expect(getShapeNodeDimensions('house')).toEqual({ width: 236, height: 124 })
    })

    it('falls back unknown shapes to the task silhouette', () => {
        expect(normalizeWorkflowNodeShape('ellipse')).toBe('box')
        expect(getReactFlowNodeTypeForShape('ellipse')).toBe('taskNode')
        expect(getShapeNodeDimensions('ellipse')).toEqual({ width: 220, height: 110 })
    })

    it('warns when a built-in handler override conflicts with the declared shape', () => {
        expect(getShapeTypeMismatchWarning('box', 'wait.human')).toContain('Shape box normally maps to codergen')
        expect(getShapeTypeMismatchWarning('hexagon', 'wait.human')).toBeNull()
        expect(getShapeTypeMismatchWarning('hexagon', 'custom.handler')).toBeNull()
        expect(getShapeTypeMismatchWarning('hexagon', '')).toBeNull()
    })
})
