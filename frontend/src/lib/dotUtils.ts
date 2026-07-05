import type { Edge, Node } from '@xyflow/react'

import type { GraphAttrs } from '@/store'

import {
    buildCanonicalFlowModelFromEditorState,
    type CanonicalDefaultsScope,
    type CanonicalSubgraph,
    generateDotFromCanonicalFlowModel,
    sanitizeGraphId as canonicalSanitizeGraphId,
} from './canonicalFlowModel.js'

export interface DotSerializationContext {
    defaults?: Partial<CanonicalDefaultsScope>
    subgraphs?: CanonicalSubgraph[]
}

let dotSerializationContext: DotSerializationContext = {}

export function setDotSerializationContext(context?: DotSerializationContext | null): void {
    if (!context) {
        dotSerializationContext = {}
        return
    }
    dotSerializationContext = {
        defaults: context.defaults,
        subgraphs: context.subgraphs,
    }
}

export function clearDotSerializationContext(): void {
    dotSerializationContext = {}
}

export function generateDot(
    flowName: string,
    nodes: Node[],
    edges: Edge[],
    graphAttrs: GraphAttrs = {},
    context?: DotSerializationContext,
): string {
    const serializationContext = context ?? dotSerializationContext
    const canonicalModel = buildCanonicalFlowModelFromEditorState(flowName, {
        nodes,
        edges,
        graphAttrs,
        defaults: serializationContext.defaults,
        subgraphs: serializationContext.subgraphs,
    })
    return generateDotFromCanonicalFlowModel(flowName, canonicalModel)
}

export function sanitizeGraphId(flowName: string): string {
    return canonicalSanitizeGraphId(flowName)
}
