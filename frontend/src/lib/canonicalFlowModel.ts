import type { Edge, Node } from '@xyflow/react'

import type { GraphAttrs } from '@/store'

export type CanonicalAttrValue = string | number | boolean | null
export type CanonicalAttrMap = Record<string, CanonicalAttrValue>

export interface CanonicalDefaultsScope {
    node: CanonicalAttrMap
    edge: CanonicalAttrMap
}

export interface CanonicalSubgraph {
    id: string | null
    attrs: CanonicalAttrMap
    nodeIds: string[]
    defaults: CanonicalDefaultsScope
    subgraphs: CanonicalSubgraph[]
}

export interface CanonicalFlowNode {
    id: string
    attrs: CanonicalAttrMap
}

export interface CanonicalFlowEdge {
    source: string
    target: string
    attrs: CanonicalAttrMap
}

export interface CanonicalFlowModel {
    graphId: string
    graphAttrs: CanonicalAttrMap
    nodes: CanonicalFlowNode[]
    edges: CanonicalFlowEdge[]
    defaults: CanonicalDefaultsScope
    subgraphs: CanonicalSubgraph[]
    rawDot: string | null
}

export interface CanonicalPreviewGraphPayload {
    nodes: Array<Record<string, unknown>>
    edges: Array<Record<string, unknown>>
    graph_attrs?: Record<string, unknown> | null
    defaults?: Record<string, unknown> | null
    subgraphs?: unknown[] | null
}

export interface CanonicalModelBuildOptions {
    rawDot?: string | null
    defaults?: Partial<CanonicalDefaultsScope>
    subgraphs?: CanonicalSubgraph[]
}

export interface CanonicalEditorStateInput extends CanonicalModelBuildOptions {
    nodes: Node[]
    edges: Edge[]
    graphAttrs: GraphAttrs
}

const PREVIEW_NODE_META_KEYS = new Set<string>(['id'])
const PREVIEW_EDGE_META_KEYS = new Set<string>(['from', 'to', 'source', 'target'])
const EPHEMERAL_NODE_ATTR_KEYS = new Set<string>(['status'])

function isCanonicalAttrValue(value: unknown): value is CanonicalAttrValue {
    return value === null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean'
}

function asRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object') {
        return null
    }
    return value as Record<string, unknown>
}

function cloneCanonicalAttrMap(
    attrs: Record<string, unknown> | null | undefined,
    excludedKeys?: Set<string>,
): CanonicalAttrMap {
    const record = asRecord(attrs)
    if (!record) {
        return {}
    }

    const cloned: CanonicalAttrMap = {}
    Object.entries(record).forEach(([key, value]) => {
        if (excludedKeys?.has(key)) {
            return
        }
        if (isCanonicalAttrValue(value)) {
            cloned[key] = value
        }
    })
    return cloned
}

function cloneDefaultsScope(defaults?: Partial<CanonicalDefaultsScope>): CanonicalDefaultsScope {
    return {
        node: cloneCanonicalAttrMap(defaults?.node),
        edge: cloneCanonicalAttrMap(defaults?.edge),
    }
}

function parseDefaultsScope(defaults: unknown): CanonicalDefaultsScope {
    const defaultsRecord = asRecord(defaults)
    return {
        node: cloneCanonicalAttrMap(asRecord(defaultsRecord?.node)),
        edge: cloneCanonicalAttrMap(asRecord(defaultsRecord?.edge)),
    }
}

function parseNodeIds(nodeIdsValue: unknown): string[] {
    if (!Array.isArray(nodeIdsValue)) {
        return []
    }
    return nodeIdsValue.filter((nodeId): nodeId is string => typeof nodeId === 'string')
}

function parseCanonicalSubgraph(subgraphValue: unknown): CanonicalSubgraph | null {
    const record = asRecord(subgraphValue)
    if (!record) {
        return null
    }

    const nestedValues = Array.isArray(record.subgraphs) ? record.subgraphs : []
    const nestedSubgraphs = nestedValues
        .map((nestedSubgraphValue) => parseCanonicalSubgraph(nestedSubgraphValue))
        .filter((subgraph): subgraph is CanonicalSubgraph => subgraph !== null)

    return {
        id: typeof record.id === 'string' ? record.id : null,
        attrs: cloneCanonicalAttrMap(asRecord(record.attrs)),
        nodeIds: parseNodeIds(record.nodeIds ?? record.node_ids),
        defaults: parseDefaultsScope(record.defaults),
        subgraphs: nestedSubgraphs,
    }
}

function parseCanonicalSubgraphs(subgraphsValue: unknown): CanonicalSubgraph[] {
    if (!Array.isArray(subgraphsValue)) {
        return []
    }
    return subgraphsValue
        .map((subgraphValue) => parseCanonicalSubgraph(subgraphValue))
        .filter((subgraph): subgraph is CanonicalSubgraph => subgraph !== null)
}

function cloneSubgraph(subgraph: CanonicalSubgraph): CanonicalSubgraph {
    return {
        id: subgraph.id,
        attrs: cloneCanonicalAttrMap(subgraph.attrs),
        nodeIds: [...subgraph.nodeIds],
        defaults: cloneDefaultsScope(subgraph.defaults),
        subgraphs: subgraph.subgraphs.map(cloneSubgraph),
    }
}

function normalizeDotId(rawId: string): string {
    const trimmed = rawId.trim()
    if (trimmed.startsWith('"') && trimmed.endsWith('"')) {
        const inner = trimmed.slice(1, -1)
        return inner
            .replace(/\\"/g, '"')
            .replace(/\\\\/g, '\\')
    }
    return trimmed
}

function extractNodeIdsWithExplicitLabels(rawDot?: string | null): Set<string> {
    if (!rawDot) {
        return new Set<string>()
    }

    const nodeIdsWithExplicitLabels = new Set<string>()
    const nodeStatementPattern = /(^|[\r\n])\s*("(?:[^"\\]|\\.)+"|[A-Za-z_][A-Za-z0-9_]*)\s*\[([\s\S]*?)\][^\S\r\n]*;?/g
    let match = nodeStatementPattern.exec(rawDot)
    while (match !== null) {
        const nodeId = normalizeDotId(match[2])
        const attrsBody = match[3]
        if (
            nodeId !== 'graph'
            && nodeId !== 'node'
            && nodeId !== 'edge'
            && /\blabel\s*=/.test(attrsBody)
        ) {
            nodeIdsWithExplicitLabels.add(nodeId)
        }
        match = nodeStatementPattern.exec(rawDot)
    }

    return nodeIdsWithExplicitLabels
}

export function buildCanonicalFlowModelFromPreviewGraph(
    graphId: string,
    graph: CanonicalPreviewGraphPayload,
    options?: CanonicalModelBuildOptions,
): CanonicalFlowModel {
    const nodeIdsWithExplicitLabels = extractNodeIdsWithExplicitLabels(options?.rawDot)
    const nodes: CanonicalFlowNode[] = graph.nodes.flatMap((nodePayload) => {
        const nodeId = typeof nodePayload.id === 'string' ? nodePayload.id : null
        if (!nodeId) {
            return []
        }
        const attrs = cloneCanonicalAttrMap(nodePayload, PREVIEW_NODE_META_KEYS)
        if (attrs.label === nodeId && !nodeIdsWithExplicitLabels.has(nodeId)) {
            delete attrs.label
        }
        return [{
            id: nodeId,
            attrs,
        }]
    })

    const edges: CanonicalFlowEdge[] = graph.edges.flatMap((edgePayload) => {
        const source = typeof edgePayload.from === 'string'
            ? edgePayload.from
            : typeof edgePayload.source === 'string'
                ? edgePayload.source
                : null
        const target = typeof edgePayload.to === 'string'
            ? edgePayload.to
            : typeof edgePayload.target === 'string'
                ? edgePayload.target
                : null
        if (!source || !target) {
            return []
        }
        return [{
            source,
            target,
            attrs: cloneCanonicalAttrMap(edgePayload, PREVIEW_EDGE_META_KEYS),
        }]
    })

    return {
        graphId,
        graphAttrs: cloneCanonicalAttrMap(graph.graph_attrs),
        nodes,
        edges,
        defaults: cloneDefaultsScope(options?.defaults ?? parseDefaultsScope(graph.defaults)),
        subgraphs: (options?.subgraphs ?? parseCanonicalSubgraphs(graph.subgraphs)).map(cloneSubgraph),
        rawDot: options?.rawDot ?? null,
    }
}

export function buildCanonicalFlowModelFromEditorState(
    graphId: string,
    input: CanonicalEditorStateInput,
): CanonicalFlowModel {
    const nodes: CanonicalFlowNode[] = input.nodes.map((node) => {
        return {
            id: node.id,
            attrs: cloneCanonicalAttrMap(asRecord(node.data), EPHEMERAL_NODE_ATTR_KEYS),
        }
    })

    const edges: CanonicalFlowEdge[] = input.edges.map((edge) => {
        return {
            source: edge.source,
            target: edge.target,
            attrs: cloneCanonicalAttrMap(asRecord(edge.data)),
        }
    })

    return {
        graphId,
        graphAttrs: cloneCanonicalAttrMap(input.graphAttrs),
        nodes,
        edges,
        defaults: cloneDefaultsScope(input.defaults),
        subgraphs: (input.subgraphs ?? []).map(cloneSubgraph),
        rawDot: input.rawDot ?? null,
    }
}

function escapeDotString(value: string): string {
    return value
        .replace(/\\/g, '\\\\')
        .replace(/"/g, '\\"')
        .replace(/\n/g, '\\n')
}

function formatAttrValue(value: string): string {
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(value)) {
        return value
    }
    return `"${escapeDotString(value)}"`
}

function readStringAttr(attrs: CanonicalAttrMap, key: string): string {
    const value = attrs[key]
    return typeof value === 'string' ? value : ''
}

function readStringOrNumberAttr(attrs: CanonicalAttrMap, key: string): string | number | '' {
    const value = attrs[key]
    if (typeof value === 'string' || typeof value === 'number') {
        return value
    }
    return ''
}

function readExplicitBooleanAttr(attrs: CanonicalAttrMap, key: string): boolean | null {
    const value = attrs[key]
    if (value === true || value === 'true') {
        return true
    }
    if (value === false || value === 'false') {
        return false
    }
    return null
}

function formatExplicitBooleanAttr(key: string, value: boolean | null): string {
    if (value === null) {
        return ''
    }
    return `${key}=${value ? 'true' : 'false'}`
}

function formatIntAttr(key: string, value: string | number): string {
    if (value === '' || value === null || value === undefined) return ''
    const parsed = typeof value === 'number' ? Math.floor(value) : parseInt(value, 10)
    if (Number.isNaN(parsed)) return ''
    return `${key}=${parsed}`
}

function formatGraphAttr(key: string, value?: string): string {
    if (!value) return ''
    return `${key}="${escapeDotString(value)}"`
}

function formatDurationAttr(key: string, value: string): string {
    const trimmed = value.trim()
    if (trimmed === '') return ''
    if (/^\d+(ms|s|m|h|d)$/.test(trimmed)) {
        return `${key}=${trimmed}`
    }
    return `${key}="${escapeDotString(trimmed)}"`
}

export function sanitizeGraphId(flowName: string): string {
    const raw = flowName.replace(/\.dot$/i, '')
    const replaced = raw.replace(/[^A-Za-z0-9_]/g, '_')
    const normalized = replaced.length > 0 ? replaced : 'flow'
    if (/^[A-Za-z_]/.test(normalized)) {
        return normalized
    }
    return `_${normalized}`
}

const KNOWN_GRAPH_ATTR_KEYS = new Set<string>([
    'goal',
    'label',
    'model_stylesheet',
    'default_max_retry',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool_hooks.pre',
    'tool_hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_reasoning_effort',
])

const KNOWN_NODE_ATTR_KEYS = new Set<string>([
    'label',
    'shape',
    'prompt',
    'tool_command',
    'tool_hooks.pre',
    'tool_hooks.post',
    'join_policy',
    'error_policy',
    'max_parallel',
    'type',
    'max_retries',
    'goal_gate',
    'retry_target',
    'fallback_retry_target',
    'fidelity',
    'thread_id',
    'class',
    'timeout',
    'llm_model',
    'llm_provider',
    'reasoning_effort',
    'auto_status',
    'allow_partial',
    'manager.poll_interval',
    'manager.max_cycles',
    'manager.stop_condition',
    'manager.actions',
    'human.default_choice',
])

const KNOWN_EDGE_ATTR_KEYS = new Set<string>([
    'label',
    'condition',
    'weight',
    'fidelity',
    'thread_id',
    'loop_restart',
])

function formatCanonicalAttrEntry(key: string, value: CanonicalAttrValue): string {
    if (value === null) {
        return ''
    }
    if (typeof value === 'number') {
        if (!Number.isFinite(value)) {
            return ''
        }
        return `${key}=${value}`
    }
    if (typeof value === 'boolean') {
        return `${key}=${value ? 'true' : 'false'}`
    }
    return `${key}=${formatAttrValue(value)}`
}

function formatCanonicalAttrEntries(attrs: CanonicalAttrMap, excludedKeys?: Set<string>): string[] {
    return Object.entries(attrs)
        .filter(([key, value]) => value !== null && !excludedKeys?.has(key))
        .sort(([leftKey], [rightKey]) => leftKey.localeCompare(rightKey))
        .map(([key, value]) => formatCanonicalAttrEntry(key, value))
        .filter((entry) => entry !== '')
}

function appendDefaultsScopeDot(dot: string, defaults: CanonicalDefaultsScope, indent = '  '): string {
    const nodeDefaults = formatCanonicalAttrEntries(defaults.node)
    const edgeDefaults = formatCanonicalAttrEntries(defaults.edge)

    if (nodeDefaults.length > 0) {
        dot += `${indent}node [${nodeDefaults.join(', ')}];\n`
    }
    if (edgeDefaults.length > 0) {
        dot += `${indent}edge [${edgeDefaults.join(', ')}];\n`
    }
    return dot
}

function appendSubgraphDot(dot: string, subgraph: CanonicalSubgraph, indent = '  '): string {
    const innerIndent = `${indent}  `
    const header = subgraph.id ? `subgraph ${subgraph.id}` : 'subgraph'
    dot += `${indent}${header} {\n`

    const subgraphAttrs = formatCanonicalAttrEntries(subgraph.attrs)
    if (subgraphAttrs.length > 0) {
        dot += `${innerIndent}graph [${subgraphAttrs.join(', ')}];\n`
    }

    dot = appendDefaultsScopeDot(dot, subgraph.defaults, innerIndent)

    subgraph.nodeIds.forEach((nodeId) => {
        if (nodeId.trim().length > 0) {
            dot += `${innerIndent}${nodeId};\n`
        }
    })

    subgraph.subgraphs.forEach((nestedSubgraph) => {
        dot = appendSubgraphDot(dot, nestedSubgraph, innerIndent)
    })

    dot += `${indent}}\n`
    return dot
}

export function generateDotFromCanonicalFlowModel(flowName: string, model: CanonicalFlowModel): string {
    const graphAttrs = model.graphAttrs
    let dot = `digraph ${sanitizeGraphId(flowName)} {\n`

    const graphAttrLines = [
        formatGraphAttr('goal', readStringAttr(graphAttrs, 'goal')),
        formatGraphAttr('label', readStringAttr(graphAttrs, 'label')),
        formatGraphAttr('model_stylesheet', readStringAttr(graphAttrs, 'model_stylesheet')),
        formatIntAttr('default_max_retry', readStringOrNumberAttr(graphAttrs, 'default_max_retry')),
        formatGraphAttr('retry_target', readStringAttr(graphAttrs, 'retry_target')),
        formatGraphAttr('fallback_retry_target', readStringAttr(graphAttrs, 'fallback_retry_target')),
        formatGraphAttr('default_fidelity', readStringAttr(graphAttrs, 'default_fidelity')),
        formatGraphAttr('stack.child_dotfile', readStringAttr(graphAttrs, 'stack.child_dotfile')),
        formatGraphAttr('stack.child_workdir', readStringAttr(graphAttrs, 'stack.child_workdir')),
        formatGraphAttr('tool_hooks.pre', readStringAttr(graphAttrs, 'tool_hooks.pre')),
        formatGraphAttr('tool_hooks.post', readStringAttr(graphAttrs, 'tool_hooks.post')),
        formatGraphAttr('ui_default_llm_model', readStringAttr(graphAttrs, 'ui_default_llm_model')),
        formatGraphAttr('ui_default_llm_provider', readStringAttr(graphAttrs, 'ui_default_llm_provider')),
        formatGraphAttr('ui_default_reasoning_effort', readStringAttr(graphAttrs, 'ui_default_reasoning_effort')),
        ...formatCanonicalAttrEntries(graphAttrs, KNOWN_GRAPH_ATTR_KEYS),
    ].filter(Boolean)

    if (graphAttrLines.length > 0) {
        dot += `  graph [${graphAttrLines.join(', ')}];\n`
    }

    dot = appendDefaultsScopeDot(dot, model.defaults)

    model.nodes.forEach((node) => {
        const attrs = node.attrs
        const hasLabelAttr = Object.prototype.hasOwnProperty.call(attrs, 'label')
        const labelValue = readStringAttr(attrs, 'label')
        const shapeValue = readStringAttr(attrs, 'shape')
        const promptValue = readStringAttr(attrs, 'prompt')
        const toolCommandValue = readStringAttr(attrs, 'tool_command')
        const toolHooksPreValue = readStringAttr(attrs, 'tool_hooks.pre')
        const toolHooksPostValue = readStringAttr(attrs, 'tool_hooks.post')
        const joinPolicyValue = readStringAttr(attrs, 'join_policy')
        const errorPolicyValue = readStringAttr(attrs, 'error_policy')
        const maxParallelValue = readStringOrNumberAttr(attrs, 'max_parallel')
        const typeValue = readStringAttr(attrs, 'type')
        const maxRetriesValue = readStringOrNumberAttr(attrs, 'max_retries')
        const goalGateValue = readExplicitBooleanAttr(attrs, 'goal_gate')
        const retryTargetValue = readStringAttr(attrs, 'retry_target')
        const fallbackRetryTargetValue = readStringAttr(attrs, 'fallback_retry_target')
        const fidelityValue = readStringAttr(attrs, 'fidelity')
        const threadIdValue = readStringAttr(attrs, 'thread_id')
        const classValue = readStringAttr(attrs, 'class')
        const timeoutValue = readStringAttr(attrs, 'timeout')
        const llmModelValue = readStringAttr(attrs, 'llm_model')
        const llmProviderValue = readStringAttr(attrs, 'llm_provider')
        const reasoningEffortValue = readStringAttr(attrs, 'reasoning_effort')
        const autoStatusValue = readExplicitBooleanAttr(attrs, 'auto_status')
        const allowPartialValue = readExplicitBooleanAttr(attrs, 'allow_partial')
        const managerPollIntervalValue = readStringAttr(attrs, 'manager.poll_interval')
        const managerMaxCyclesValue = readStringOrNumberAttr(attrs, 'manager.max_cycles')
        const managerStopConditionValue = readStringAttr(attrs, 'manager.stop_condition')
        const managerActionsValue = readStringAttr(attrs, 'manager.actions')
        const humanDefaultChoiceValue = readStringAttr(attrs, 'human.default_choice')

        const label = hasLabelAttr ? `label="${escapeDotString(labelValue)}"` : ''
        const shape = shapeValue ? `shape=${formatAttrValue(shapeValue)}` : ''
        const prompt = promptValue ? `prompt="${escapeDotString(promptValue)}"` : ''
        const toolCommand = toolCommandValue ? `tool_command="${escapeDotString(toolCommandValue)}"` : ''
        const toolHooksPre = toolHooksPreValue ? `tool_hooks.pre="${escapeDotString(toolHooksPreValue)}"` : ''
        const toolHooksPost = toolHooksPostValue ? `tool_hooks.post="${escapeDotString(toolHooksPostValue)}"` : ''
        const joinPolicy = joinPolicyValue ? `join_policy=${formatAttrValue(joinPolicyValue)}` : ''
        const errorPolicy = errorPolicyValue ? `error_policy=${formatAttrValue(errorPolicyValue)}` : ''
        const maxParallel = formatIntAttr('max_parallel', maxParallelValue)

        const nodeAttrs = [
            label,
            shape,
            prompt,
            toolCommand,
            toolHooksPre,
            toolHooksPost,
            joinPolicy,
            errorPolicy,
            maxParallel,
            typeValue ? `type=${formatAttrValue(typeValue)}` : '',
            formatIntAttr('max_retries', maxRetriesValue),
            formatExplicitBooleanAttr('goal_gate', goalGateValue),
            retryTargetValue ? `retry_target=${formatAttrValue(retryTargetValue)}` : '',
            fallbackRetryTargetValue ? `fallback_retry_target=${formatAttrValue(fallbackRetryTargetValue)}` : '',
            fidelityValue ? `fidelity=${formatAttrValue(fidelityValue)}` : '',
            threadIdValue ? `thread_id="${escapeDotString(threadIdValue)}"` : '',
            classValue ? `class="${escapeDotString(classValue)}"` : '',
            timeoutValue ? formatDurationAttr('timeout', timeoutValue) : '',
            llmModelValue ? `llm_model=${formatAttrValue(llmModelValue)}` : '',
            llmProviderValue ? `llm_provider=${formatAttrValue(llmProviderValue)}` : '',
            reasoningEffortValue ? `reasoning_effort=${formatAttrValue(reasoningEffortValue)}` : '',
            formatExplicitBooleanAttr('auto_status', autoStatusValue),
            formatExplicitBooleanAttr('allow_partial', allowPartialValue),
            managerPollIntervalValue ? formatDurationAttr('manager.poll_interval', managerPollIntervalValue) : '',
            formatIntAttr('manager.max_cycles', managerMaxCyclesValue),
            managerStopConditionValue ? `manager.stop_condition="${escapeDotString(managerStopConditionValue)}"` : '',
            managerActionsValue ? `manager.actions="${escapeDotString(managerActionsValue)}"` : '',
            humanDefaultChoiceValue ? `human.default_choice=${formatAttrValue(humanDefaultChoiceValue)}` : '',
            ...formatCanonicalAttrEntries(attrs, KNOWN_NODE_ATTR_KEYS),
        ].filter(Boolean).join(', ')

        dot += `  ${node.id} [${nodeAttrs}];\n`
    })

    model.edges.forEach((edge) => {
        const attrs = edge.attrs
        const labelValue = readStringAttr(attrs, 'label')
        const conditionValue = readStringAttr(attrs, 'condition')
        const weightValue = readStringOrNumberAttr(attrs, 'weight')
        const fidelityValue = readStringAttr(attrs, 'fidelity')
        const threadIdValue = readStringAttr(attrs, 'thread_id')
        const loopRestartValue = readExplicitBooleanAttr(attrs, 'loop_restart')

        const edgeAttrs = [
            labelValue ? `label="${escapeDotString(labelValue)}"` : '',
            conditionValue ? `condition="${escapeDotString(conditionValue)}"` : '',
            formatIntAttr('weight', weightValue),
            fidelityValue ? `fidelity=${formatAttrValue(fidelityValue)}` : '',
            threadIdValue ? `thread_id="${escapeDotString(threadIdValue)}"` : '',
            formatExplicitBooleanAttr('loop_restart', loopRestartValue),
            ...formatCanonicalAttrEntries(attrs, KNOWN_EDGE_ATTR_KEYS),
        ].filter(Boolean).join(', ')

        if (edgeAttrs) {
            dot += `  ${edge.source} -> ${edge.target} [${edgeAttrs}];\n`
        } else {
            dot += `  ${edge.source} -> ${edge.target};\n`
        }
    })

    model.subgraphs.forEach((subgraph) => {
        dot = appendSubgraphDot(dot, subgraph)
    })

    dot += '}\n'
    return dot
}
