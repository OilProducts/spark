import type { CSSProperties } from 'react'

export type WorkflowNodeShape =
    | 'Mdiamond'
    | 'Msquare'
    | 'box'
    | 'hexagon'
    | 'diamond'
    | 'component'
    | 'tripleoctagon'
    | 'parallelogram'
    | 'house'

export type WorkflowNodeType =
    | 'startNode'
    | 'exitNode'
    | 'taskNode'
    | 'humanGateNode'
    | 'conditionalNode'
    | 'parallelNode'
    | 'fanInNode'
    | 'toolNode'
    | 'managerNode'

export type WorkflowNodeDimensions = {
    width: number
    height: number
}

export const WORKFLOW_NODE_SHAPE_OPTIONS: Array<{ value: WorkflowNodeShape; label: string }> = [
    { value: 'box', label: 'Codergen (Task)' },
    { value: 'hexagon', label: 'Wait for Human' },
    { value: 'diamond', label: 'Condition' },
    { value: 'component', label: 'Parallel (Fan Out)' },
    { value: 'tripleoctagon', label: 'Parallel (Fan In)' },
    { value: 'parallelogram', label: 'Tool' },
    { value: 'house', label: 'Manager Loop' },
    { value: 'Mdiamond', label: 'Start Node' },
    { value: 'Msquare', label: 'End Node' },
]

const BOX_DIMENSIONS: WorkflowNodeDimensions = { width: 220, height: 110 }

const SHAPE_NODE_TYPE: Record<WorkflowNodeShape, WorkflowNodeType> = {
    Mdiamond: 'startNode',
    Msquare: 'exitNode',
    box: 'taskNode',
    hexagon: 'humanGateNode',
    diamond: 'conditionalNode',
    component: 'parallelNode',
    tripleoctagon: 'fanInNode',
    parallelogram: 'toolNode',
    house: 'managerNode',
}

const SHAPE_DIMENSIONS: Record<WorkflowNodeShape, WorkflowNodeDimensions> = {
    Mdiamond: { width: 168, height: 96 },
    Msquare: { width: 168, height: 96 },
    box: BOX_DIMENSIONS,
    hexagon: { width: 228, height: 116 },
    diamond: { width: 176, height: 104 },
    component: { width: 236, height: 116 },
    tripleoctagon: { width: 236, height: 116 },
    parallelogram: { width: 228, height: 116 },
    house: { width: 236, height: 124 },
}

export function isWorkflowNodeShape(value?: string | null): value is WorkflowNodeShape {
    return Boolean(value && value in SHAPE_NODE_TYPE)
}

export function normalizeWorkflowNodeShape(value?: string | null): WorkflowNodeShape {
    return isWorkflowNodeShape(value) ? value : 'box'
}

export function getReactFlowNodeTypeForShape(value?: string | null): WorkflowNodeType {
    return SHAPE_NODE_TYPE[normalizeWorkflowNodeShape(value)]
}

export function getShapeNodeDimensions(value?: string | null): WorkflowNodeDimensions {
    return SHAPE_DIMENSIONS[normalizeWorkflowNodeShape(value)]
}

export function getShapeNodeStyle(value?: string | null): CSSProperties {
    const { width, height } = getShapeNodeDimensions(value)
    return { width, height }
}

export function getNodeStyleDimension(styleValue: unknown): number | null {
    if (typeof styleValue === 'number' && Number.isFinite(styleValue)) {
        return styleValue
    }
    if (typeof styleValue === 'string') {
        const parsed = Number.parseFloat(styleValue)
        return Number.isFinite(parsed) ? parsed : null
    }
    return null
}
