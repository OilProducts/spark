import type { Edge, Node } from '@xyflow/react'

import type { FlowDefinitionMetadata } from '@/store'

import {
    buildCanonicalFlowModelFromEditorState,
    type CanonicalFlowDefinition,
    generateFlowYamlFromCanonicalFlowModel,
    sanitizeFlowId as canonicalSanitizeFlowId,
} from './canonicalFlowModel.js'

export interface FlowYamlSerializationContext {
    flow?: CanonicalFlowDefinition | null
}

let flowYamlSerializationContext: FlowYamlSerializationContext = {}

export function setFlowYamlSerializationContext(context?: FlowYamlSerializationContext | null): void {
    if (!context) {
        flowYamlSerializationContext = {}
        return
    }
    flowYamlSerializationContext = {
        flow: context.flow,
    }
}

export function clearFlowYamlSerializationContext(): void {
    flowYamlSerializationContext = {}
}

export function generateFlowYaml(
    flowName: string,
    nodes: Node[],
    edges: Edge[],
    flowMetadata: FlowDefinitionMetadata = {},
    context?: FlowYamlSerializationContext,
): string {
    const serializationContext = context ?? flowYamlSerializationContext
    const canonicalModel = buildCanonicalFlowModelFromEditorState(flowName, {
        nodes,
        edges,
        flowMetadata,
        flow: serializationContext.flow,
    })
    return generateFlowYamlFromCanonicalFlowModel(flowName, canonicalModel)
}

export function sanitizeFlowId(flowName: string): string {
    return canonicalSanitizeFlowId(flowName)
}
