import type { Edge, Node } from '@xyflow/react'

import type { FlowDefinitionMetadata } from '@/store'
import { EDGE_RENDER_ROUTE_KEY } from './flowLayoutConstants.js'

export type CanonicalAttrValue =
    | string
    | number
    | boolean
    | null
    | CanonicalAttrValue[]
    | { [key: string]: CanonicalAttrValue }
export type CanonicalAttrMap = Record<string, CanonicalAttrValue>
export type CanonicalFlowDefinition = Record<string, unknown>

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
    flowMetadata: CanonicalAttrMap
    nodes: CanonicalFlowNode[]
    edges: CanonicalFlowEdge[]
    rawYaml: string | null
    flow: CanonicalFlowDefinition | null
}

export interface CanonicalPreviewGraphPayload {
    nodes: Array<Record<string, unknown>>
    edges: Array<Record<string, unknown>>
    metadata?: Record<string, unknown> | null
    child_previews?: Record<string, unknown> | null
}

export interface CanonicalModelBuildOptions {
    rawYaml?: string | null
    flow?: CanonicalFlowDefinition | null
}

export interface CanonicalEditorStateInput extends CanonicalModelBuildOptions {
    nodes: Node[]
    edges: Edge[]
    flowMetadata: FlowDefinitionMetadata
}

const PREVIEW_NODE_META_KEYS = new Set<string>(['id'])
const PREVIEW_EDGE_META_KEYS = new Set<string>(['from', 'to', 'source', 'target'])
const EPHEMERAL_NODE_ATTR_KEYS = new Set<string>(['status'])
const EPHEMERAL_EDGE_ATTR_KEYS = new Set<string>([EDGE_RENDER_ROUTE_KEY])
const UNSUPPORTED_NODE_ATTR_KEYS = new Set<string>(['human.default_choice'])
const PREVIEW_NODE_EXCLUDED_ATTR_KEYS = new Set<string>([
    ...PREVIEW_NODE_META_KEYS,
    ...UNSUPPORTED_NODE_ATTR_KEYS,
])
const EDITOR_NODE_EXCLUDED_ATTR_KEYS = new Set<string>([
    ...EPHEMERAL_NODE_ATTR_KEYS,
    ...UNSUPPORTED_NODE_ATTR_KEYS,
])
const FLOW_METADATA_CORE_KEYS = new Set([
    'schema_version',
    'id',
    'title',
    'description',
    'goal',
    'inputs',
    'max_retries',
    'fidelity',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
])
const NODE_EXTENSION_CORE_KEYS = new Set([
    'kind',
    'config',
    'context',
    'contracts',
    'runtime',
    'manager',
    'retry',
    'execution',
    'ui',
    'extensions',
    'label',
    'description',
    'prompt',
    'options',
    'tool.command',
    'flow_ref',
    'input_map',
    'decisions',
    'stack.child_flow_ref',
    'retry_policy',
    'max_retries',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
])
const EDGE_EXTENSION_CORE_KEYS = new Set(['label', 'condition', 'weight', 'transition', 'extensions'])
const VALID_NODE_KINDS = new Set([
    'start',
    'exit',
    'agent_task',
    'human_gate',
    'conditional',
    'parallel',
    'fan_in',
    'tool',
    'subflow',
])
const DEPRECATED_DOT_METADATA_KEYS = new Set([
    'label',
    'spark.title',
    'spark.description',
    'spark.launch_inputs',
    'default_max_retries',
    'default_fidelity',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_llm_profile',
    'ui_default_reasoning_effort',
    'model_stylesheet',
    'stack.child_yamlfile',
    'stack.child_workdir',
])
const DEPRECATED_DOT_NODE_KEYS = new Set([
    'shape',
    'type',
    'stack.child_yamlfile',
])

function isCanonicalAttrValue(value: unknown): value is CanonicalAttrValue {
    if (value === null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
        return true
    }
    if (Array.isArray(value)) {
        return value.every(isCanonicalAttrValue)
    }
    if (value && typeof value === 'object') {
        return Object.values(value as Record<string, unknown>).every(isCanonicalAttrValue)
    }
    return false
}

function asRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object') {
        return null
    }
    return value as Record<string, unknown>
}

function cloneCanonicalAttrMap(
    attrs: unknown,
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
            cloned[key] = cloneCanonicalAttrValue(value)
        }
    })
    return cloned
}

function cloneCanonicalAttrValue(value: CanonicalAttrValue): CanonicalAttrValue {
    if (Array.isArray(value)) {
        return value.map(cloneCanonicalAttrValue)
    }
    if (value && typeof value === 'object') {
        return Object.fromEntries(
            Object.entries(value).map(([key, item]) => [key, cloneCanonicalAttrValue(item)]),
        )
    }
    return value
}

function cloneFlowDefinition(flow?: CanonicalFlowDefinition | null): CanonicalFlowDefinition | null {
    if (!flow) {
        return null
    }
    return JSON.parse(JSON.stringify(flow)) as CanonicalFlowDefinition
}

function canonicalizeFlowMetadata(attrs: CanonicalAttrMap): CanonicalAttrMap {
    return { ...attrs }
}

function typedFlowNode(flow: CanonicalFlowDefinition | null, nodeId: string): Record<string, unknown> | null {
    return asRecord(asRecord(flow?.nodes)?.[nodeId])
}

function typedNodeConfig(node: Record<string, unknown> | null): Record<string, unknown> | null {
    return asRecord(node?.config)
}

function typedFlowEdgeByIndex(flow: CanonicalFlowDefinition | null, index: number): Record<string, unknown> | null {
    const edges = Array.isArray(flow?.edges) ? flow.edges : []
    return asRecord(edges[index])
}

function nodeKindFromPayload(payload: Record<string, unknown>, fallbackNode?: Record<string, unknown> | null): string {
    const fallbackConfig = typedNodeConfig(fallbackNode ?? null)
    const kind = typeof payload.kind === 'string'
        ? payload.kind
        : typeof fallbackNode?.kind === 'string'
            ? fallbackNode.kind
            : typeof fallbackConfig?.kind === 'string'
                ? String(fallbackConfig.kind)
                : ''
    return VALID_NODE_KINDS.has(kind) ? kind : ''
}

function enrichNodeAttrsFromTypedPayload(
    nodePayload: Record<string, unknown>,
    fallbackNode: Record<string, unknown> | null,
): CanonicalAttrMap {
    const attrs = cloneCanonicalAttrMap(
        {
            ...(fallbackNode ?? {}),
            ...nodePayload,
        },
        PREVIEW_NODE_EXCLUDED_ATTR_KEYS,
    )
    const kind = nodeKindFromPayload(nodePayload, fallbackNode) || 'agent_task'
    const config = asRecord(attrs.config) ?? typedNodeConfig(fallbackNode)

    attrs.kind = kind

    if (config) {
        attrs.config = cloneCanonicalAttrMap(config)
        if ((kind === 'agent_task' || kind === 'human_gate') && typeof config.prompt === 'string') {
            attrs.prompt = typeof attrs.prompt === 'string' && attrs.prompt ? attrs.prompt : config.prompt
        }
        if (kind === 'human_gate' && Array.isArray(config.decisions)) {
            attrs.decisions = config.decisions as CanonicalAttrValue
        }
        if (kind === 'tool' && typeof config.command === 'string') {
            attrs['tool.command'] = typeof attrs['tool.command'] === 'string' && attrs['tool.command']
                ? attrs['tool.command']
                : config.command
        }
        if (kind === 'subflow') {
            if (typeof config.flow_ref === 'string') {
                attrs.flow_ref = config.flow_ref
            }
            if (asRecord(config.input_map)) {
                attrs.input_map = cloneCanonicalAttrMap(config.input_map)
            }
        }
        if (kind === 'parallel') {
            ;(['join_policy', 'max_parallel', 'join_k', 'join_quorum'] as const).forEach((key) => {
                if (config[key] !== undefined && isCanonicalAttrValue(config[key])) {
                    attrs[key] = cloneCanonicalAttrValue(config[key])
                }
            })
        }
    }

    const execution = asRecord(attrs.execution)
    if (execution) {
        ;(['llm_model', 'llm_provider', 'llm_profile', 'reasoning_effort'] as const).forEach((key) => {
            if (typeof execution[key] === 'string') {
                attrs[key] = execution[key]
            }
        })
    }
    const retry = asRecord(attrs.retry)
    if (retry) {
        if (typeof retry.max_retries === 'number' || typeof retry.max_retries === 'string') {
            attrs.max_retries = retry.max_retries
        }
        if (typeof retry.policy === 'string') {
            attrs.retry_policy = retry.policy
        }
    }
    const runtime = asRecord(attrs.runtime)
    if (runtime) {
        ;(['allow_partial', 'auto_status', 'goal_gate'] as const).forEach((key) => {
            if (typeof runtime[key] === 'boolean') {
                attrs[key] = runtime[key]
            }
        })
        ;(['error_policy', 'fidelity', 'thread_id', 'class', 'timeout', 'retry_target', 'fallback_retry_target'] as const).forEach((key) => {
            if (typeof runtime[key] === 'string') {
                attrs[key] = runtime[key]
            }
        })
    }
    const contracts = asRecord(attrs.contracts)
    if (contracts) {
        if (Array.isArray(contracts.reads_context)) {
            attrs['spark.reads_context'] = JSON.stringify(contracts.reads_context)
        }
        if (Array.isArray(contracts.writes_context)) {
            attrs['spark.writes_context'] = JSON.stringify(contracts.writes_context)
        }
    }
    const manager = asRecord(attrs.manager)
    if (manager) {
        const managerKeys = {
            poll_interval: 'manager.poll_interval',
            max_cycles: 'manager.max_cycles',
            stop_condition: 'manager.stop_condition',
            steer_cooldown: 'manager.steer_cooldown',
            child_autostart: 'stack.child_autostart',
        } as const
        Object.entries(managerKeys).forEach(([sourceKey, attrKey]) => {
            const value = manager[sourceKey]
            if (isCanonicalAttrValue(value)) {
                attrs[attrKey] = cloneCanonicalAttrValue(value)
            }
        })
        if (Array.isArray(manager.actions)) {
            attrs['manager.actions'] = manager.actions.join(',')
        }
    }

    return attrs
}

function flowMetadataFromFlow(flow: CanonicalFlowDefinition | null, graphMetadata: unknown): CanonicalAttrMap {
    const metadata = cloneCanonicalAttrMap(asRecord(flow?.metadata) ?? graphMetadata)
    const defaults = asRecord(flow?.defaults)
    if (typeof flow?.schema_version === 'string') metadata.schema_version = flow.schema_version
    if (typeof flow?.id === 'string') metadata.id = flow.id
    if (typeof flow?.title === 'string') metadata.title = flow.title
    if (typeof flow?.description === 'string') metadata.description = flow.description
    if (typeof flow?.goal === 'string') metadata.goal = flow.goal
    if (Array.isArray(flow?.inputs)) metadata.inputs = JSON.stringify(flow.inputs)
    if (defaults) {
        if (typeof defaults.max_retries === 'number') metadata.max_retries = defaults.max_retries
        if (typeof defaults.fidelity === 'string') metadata.fidelity = defaults.fidelity
        if (typeof defaults.llm_model === 'string') metadata.llm_model = defaults.llm_model
        if (typeof defaults.llm_provider === 'string') metadata.llm_provider = defaults.llm_provider
        if (typeof defaults.llm_profile === 'string') metadata.llm_profile = defaults.llm_profile
        if (typeof defaults.reasoning_effort === 'string') metadata.reasoning_effort = defaults.reasoning_effort
    }
    DEPRECATED_DOT_METADATA_KEYS.forEach((key) => {
        delete metadata[key]
    })
    return metadata
}

export function buildCanonicalFlowModelFromPreviewGraph(
    graphId: string,
    graph: CanonicalPreviewGraphPayload,
    options?: CanonicalModelBuildOptions,
): CanonicalFlowModel {
    const baseFlow = cloneFlowDefinition(options?.flow)
    const nodes: CanonicalFlowNode[] = graph.nodes.flatMap((nodePayload) => {
        const nodeId = typeof nodePayload.id === 'string' ? nodePayload.id : null
        if (!nodeId) {
            return []
        }
        const attrs = enrichNodeAttrsFromTypedPayload(nodePayload, typedFlowNode(baseFlow, nodeId))
        return [{
            id: nodeId,
            attrs,
        }]
    })

    const edges: CanonicalFlowEdge[] = graph.edges.flatMap((edgePayload, index) => {
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
        const fallbackEdge = typedFlowEdgeByIndex(baseFlow, index)
        return [{
            source,
            target,
            attrs: cloneCanonicalAttrMap({
                ...(fallbackEdge ?? {}),
                ...edgePayload,
            }, PREVIEW_EDGE_META_KEYS),
        }]
    })

    return {
        graphId,
        flowMetadata: canonicalizeFlowMetadata(flowMetadataFromFlow(baseFlow, graph.metadata)),
        nodes,
        edges,
        rawYaml: options?.rawYaml ?? null,
        flow: baseFlow,
    }
}

export function buildCanonicalFlowModelFromEditorState(
    graphId: string,
    input: CanonicalEditorStateInput,
): CanonicalFlowModel {
    const nodes: CanonicalFlowNode[] = input.nodes.map((node) => {
        return {
            id: node.id,
            attrs: cloneCanonicalAttrMap(asRecord(node.data), EDITOR_NODE_EXCLUDED_ATTR_KEYS),
        }
    })

    const edges: CanonicalFlowEdge[] = input.edges.map((edge) => {
        return {
            source: edge.source,
            target: edge.target,
            attrs: cloneCanonicalAttrMap(asRecord(edge.data), EPHEMERAL_EDGE_ATTR_KEYS),
        }
    })

    return {
        graphId,
        flowMetadata: canonicalizeFlowMetadata(cloneCanonicalAttrMap(input.flowMetadata)),
        nodes,
        edges,
        rawYaml: input.rawYaml ?? null,
        flow: cloneFlowDefinition(input.flow),
    }
}

function readStringAttr(attrs: CanonicalAttrMap, key: string): string {
    const value = attrs[key]
    return typeof value === 'string' ? value : ''
}

function readNumberAttr(attrs: CanonicalAttrMap, key: string): number | undefined {
    const value = attrs[key]
    if (typeof value === 'number' && Number.isFinite(value)) {
        return value
    }
    if (typeof value === 'string' && value.trim()) {
        const parsed = Number(value)
        return Number.isFinite(parsed) ? parsed : undefined
    }
    return undefined
}

function readBooleanAttr(attrs: CanonicalAttrMap, key: string): boolean | undefined {
    const value = attrs[key]
    if (typeof value === 'boolean') {
        return value
    }
    if (value === 'true') {
        return true
    }
    if (value === 'false') {
        return false
    }
    return undefined
}

export function sanitizeFlowId(flowName: string): string {
    const raw = flowName.replace(/\.(ya?ml|json)$/i, '')
    const replaced = raw.replace(/[^A-Za-z0-9_]/g, '_')
    const normalized = replaced.length > 0 ? replaced : 'flow'
    if (/^[A-Za-z_]/.test(normalized)) {
        return normalized
    }
    return `_${normalized}`
}

function yamlScalar(value: unknown): string {
    if (value === null || value === undefined) return 'null'
    if (typeof value === 'number' || typeof value === 'boolean') return String(value)
    const text = String(value)
    if (
        /^[A-Za-z0-9_./:-]+$/.test(text)
        && text !== ''
        && !/^(?:true|false|null|~)$/i.test(text)
        && !/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text)
    ) return text
    return JSON.stringify(text)
}

function yamlBlock(value: unknown, indent = 0): string {
    const pad = ' '.repeat(indent)
    if (Array.isArray(value)) {
        if (value.length === 0) return `${pad}[]`
        return value.map((item) => `${pad}- ${yamlBlock(item, indent + 2).trimStart()}`).join('\n')
    }
    if (value && typeof value === 'object') {
        const entries = Object.entries(value as Record<string, unknown>).filter(([, item]) => item !== undefined)
        if (entries.length === 0) return `${pad}{}`
        return entries.map(([key, item]) => {
            if (item && typeof item === 'object') {
                return `${pad}${key}:\n${yamlBlock(item, indent + 2)}`
            }
            return `${pad}${key}: ${yamlScalar(item)}`
        }).join('\n')
    }
    return yamlScalar(value)
}

function nodeKindFromAttrs(attrs: CanonicalAttrMap, baseNode?: Record<string, unknown> | null): string {
    const explicitKind = readStringAttr(attrs, 'kind')
    if (VALID_NODE_KINDS.has(explicitKind)) return explicitKind
    const configKind = readStringAttr(cloneCanonicalAttrMap(attrs.config), 'kind')
    if (VALID_NODE_KINDS.has(configKind)) return configKind
    const baseKind = typeof baseNode?.kind === 'string' ? baseNode.kind : ''
    if (VALID_NODE_KINDS.has(baseKind)) return baseKind
    const baseConfigKind = readStringAttr(cloneCanonicalAttrMap(asRecord(baseNode?.config)), 'kind')
    if (VALID_NODE_KINDS.has(baseConfigKind)) return baseConfigKind
    return 'agent_task'
}

function compactObject<T extends Record<string, unknown>>(value: T): T {
    Object.keys(value).forEach((key) => {
        const item = value[key]
        if (
            item === undefined
            || (Array.isArray(item) && item.length === 0)
            || (item && typeof item === 'object' && !Array.isArray(item) && Object.keys(item).length === 0)
        ) {
            delete value[key]
        }
    })
    return value
}

function parseStringArrayJson(value: unknown): string[] {
    if (!value) return []
    if (Array.isArray(value)) return value.filter((item): item is string => typeof item === 'string')
    if (typeof value !== 'string' || !value.trim()) return []
    try {
        const parsed = JSON.parse(value)
        return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string') : []
    } catch {
        return []
    }
}

function nodeConfigFromAttrs(kind: string, attrs: CanonicalAttrMap, baseConfig?: Record<string, unknown> | null): Record<string, unknown> {
    const base = { ...(baseConfig ?? {}) }
    if (kind === 'agent_task') return compactObject({ ...base, kind, prompt: readStringAttr(attrs, 'prompt') })
    if (kind === 'human_gate') {
        return compactObject({
            ...base,
            kind,
            prompt: readStringAttr(attrs, 'prompt'),
            decisions: Array.isArray(attrs.decisions) ? attrs.decisions : base.decisions,
        })
    }
    if (kind === 'tool') return compactObject({ ...base, kind, command: readStringAttr(attrs, 'tool.command') })
    if (kind === 'subflow') {
        return compactObject({
            ...base,
            kind,
            flow_ref: readStringAttr(attrs, 'flow_ref')
                || String(base.flow_ref ?? 'child.yaml'),
            input_map: asRecord(attrs.input_map) ?? asRecord(base.input_map) ?? undefined,
        })
    }
    if (kind === 'parallel') {
        return compactObject({
            ...base,
            kind,
            join_policy: readStringAttr(attrs, 'join_policy') || base.join_policy,
            max_parallel: readNumberAttr(attrs, 'max_parallel') ?? base.max_parallel,
            join_k: readNumberAttr(attrs, 'join_k') ?? base.join_k,
            join_quorum: readNumberAttr(attrs, 'join_quorum') ?? base.join_quorum,
        })
    }
    return compactObject({ ...base, kind })
}

function extensionAttrs(attrs: CanonicalAttrMap, coreKeys: Set<string>): CanonicalAttrMap {
    return Object.fromEntries(
        Object.entries(attrs).filter(([key, value]) => (
            !coreKeys.has(key)
            && !DEPRECATED_DOT_METADATA_KEYS.has(key)
            && !DEPRECATED_DOT_NODE_KEYS.has(key)
            && isCanonicalAttrValue(value)
        )),
    )
}

function flowInputsFromAttrs(attrs: CanonicalAttrMap): unknown[] {
    const rawInputs = readStringAttr(attrs, 'inputs') || readStringAttr(attrs, 'spark.launch_inputs')
    if (!rawInputs) return []
    try {
        const parsed = JSON.parse(rawInputs)
        return Array.isArray(parsed) ? parsed : []
    } catch {
        return []
    }
}

function flowDefaultsFromAttrs(attrs: CanonicalAttrMap, baseDefaults?: Record<string, unknown> | null): Record<string, unknown> {
    return compactObject({
        ...(baseDefaults ?? {}),
        max_retries: readNumberAttr(attrs, 'max_retries') ?? baseDefaults?.max_retries,
        fidelity: readStringAttr(attrs, 'fidelity') || baseDefaults?.fidelity,
        llm_model: readStringAttr(attrs, 'llm_model') || baseDefaults?.llm_model,
        llm_provider: readStringAttr(attrs, 'llm_provider') || baseDefaults?.llm_provider,
        llm_profile: readStringAttr(attrs, 'llm_profile') || baseDefaults?.llm_profile,
        reasoning_effort: readStringAttr(attrs, 'reasoning_effort') || baseDefaults?.reasoning_effort,
    })
}

function retryFromAttrs(attrs: CanonicalAttrMap, baseRetry?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseRetry ?? {}),
        policy: readStringAttr(attrs, 'retry_policy') || baseRetry?.policy,
        max_retries: readNumberAttr(attrs, 'max_retries') ?? baseRetry?.max_retries,
    })
}

function executionFromAttrs(attrs: CanonicalAttrMap, baseExecution?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseExecution ?? {}),
        llm_model: readStringAttr(attrs, 'llm_model') || baseExecution?.llm_model,
        llm_provider: readStringAttr(attrs, 'llm_provider') || baseExecution?.llm_provider,
        llm_profile: readStringAttr(attrs, 'llm_profile') || baseExecution?.llm_profile,
        reasoning_effort: readStringAttr(attrs, 'reasoning_effort') || baseExecution?.reasoning_effort,
    })
}

function runtimeFromAttrs(attrs: CanonicalAttrMap, baseRuntime?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseRuntime ?? {}),
        allow_partial: readBooleanAttr(attrs, 'allow_partial') ?? baseRuntime?.allow_partial,
        auto_status: readBooleanAttr(attrs, 'auto_status') ?? baseRuntime?.auto_status,
        goal_gate: readBooleanAttr(attrs, 'goal_gate') ?? baseRuntime?.goal_gate,
        error_policy: readStringAttr(attrs, 'error_policy') || baseRuntime?.error_policy,
        fidelity: readStringAttr(attrs, 'fidelity') || baseRuntime?.fidelity,
        thread_id: readStringAttr(attrs, 'thread_id') || baseRuntime?.thread_id,
        class: readStringAttr(attrs, 'class') || baseRuntime?.class,
        timeout: readStringAttr(attrs, 'timeout') || baseRuntime?.timeout,
        retry_target: readStringAttr(attrs, 'retry_target') || baseRuntime?.retry_target,
        fallback_retry_target: readStringAttr(attrs, 'fallback_retry_target') || baseRuntime?.fallback_retry_target,
    })
}

function contractsFromAttrs(attrs: CanonicalAttrMap, baseContracts?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseContracts ?? {}),
        reads_context: parseStringArrayJson(attrs['spark.reads_context']).length > 0
            ? parseStringArrayJson(attrs['spark.reads_context'])
            : baseContracts?.reads_context,
        writes_context: parseStringArrayJson(attrs['spark.writes_context']).length > 0
            ? parseStringArrayJson(attrs['spark.writes_context'])
            : baseContracts?.writes_context,
    })
}

function managerFromAttrs(attrs: CanonicalAttrMap, baseManager?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    const actions = readStringAttr(attrs, 'manager.actions')
    return compactObject({
        ...(baseManager ?? {}),
        poll_interval: readStringAttr(attrs, 'manager.poll_interval') || baseManager?.poll_interval,
        max_cycles: readNumberAttr(attrs, 'manager.max_cycles') ?? baseManager?.max_cycles,
        stop_condition: readStringAttr(attrs, 'manager.stop_condition') || baseManager?.stop_condition,
        actions: actions
            ? actions.split(',').map((entry) => entry.trim()).filter(Boolean)
            : baseManager?.actions,
        steer_cooldown: readStringAttr(attrs, 'manager.steer_cooldown') || baseManager?.steer_cooldown,
        child_autostart: readBooleanAttr(attrs, 'stack.child_autostart') ?? baseManager?.child_autostart,
    })
}

function mergeExtensions(
    baseExtensions: Record<string, unknown> | null,
    attrs: CanonicalAttrMap,
    coreKeys: Set<string>,
): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseExtensions ?? {}),
        ...extensionAttrs(attrs, coreKeys),
    })
}

function nodeFromCanonicalAttrs(
    nodeId: string,
    attrs: CanonicalAttrMap,
    baseNode?: Record<string, unknown> | null,
): Record<string, unknown> {
    const kind = nodeKindFromAttrs(attrs, baseNode)
    const nextNode = compactObject({
        ...(baseNode ?? {}),
        kind,
        label: readStringAttr(attrs, 'label') || String(baseNode?.label ?? nodeId),
        description: readStringAttr(attrs, 'description') || baseNode?.description,
        config: nodeConfigFromAttrs(kind, attrs, asRecord(baseNode?.config)),
        context: asRecord(attrs.context) ?? asRecord(baseNode?.context) ?? undefined,
        contracts: contractsFromAttrs(attrs, asRecord(baseNode?.contracts)),
        runtime: runtimeFromAttrs(attrs, asRecord(baseNode?.runtime)),
        manager: managerFromAttrs(attrs, asRecord(baseNode?.manager)),
        retry: retryFromAttrs(attrs, asRecord(baseNode?.retry)),
        execution: executionFromAttrs(attrs, asRecord(baseNode?.execution)),
        ui: asRecord(attrs.ui) ?? asRecord(baseNode?.ui) ?? undefined,
        extensions: mergeExtensions(asRecord(baseNode?.extensions), attrs, NODE_EXTENSION_CORE_KEYS),
    })
    return nextNode
}

function edgeFromCanonicalAttrs(
    edge: CanonicalFlowEdge,
    baseEdge?: Record<string, unknown> | null,
): Record<string, unknown> {
    return compactObject({
        ...(baseEdge ?? {}),
        from: edge.source,
        to: edge.target,
        label: readStringAttr(edge.attrs, 'label') || baseEdge?.label,
        condition: readStringAttr(edge.attrs, 'condition') || baseEdge?.condition,
        weight: readNumberAttr(edge.attrs, 'weight') ?? baseEdge?.weight,
        transition: readStringAttr(edge.attrs, 'transition') || baseEdge?.transition,
        extensions: mergeExtensions(asRecord(baseEdge?.extensions), edge.attrs, EDGE_EXTENSION_CORE_KEYS),
    })
}

export function generateFlowYamlFromCanonicalFlowModel(flowName: string, model: CanonicalFlowModel): string {
    const flowMetadata = model.flowMetadata
    const baseFlow = cloneFlowDefinition(model.flow) ?? {}
    const baseNodes = asRecord(baseFlow.nodes)
    const baseEdges = Array.isArray(baseFlow.edges) ? baseFlow.edges : []
    const nodes = Object.fromEntries(model.nodes.map((node) => {
        return [node.id, nodeFromCanonicalAttrs(node.id, node.attrs, asRecord(baseNodes?.[node.id]))]
    }))
    const edges = model.edges.map((edge, index) => edgeFromCanonicalAttrs(edge, asRecord(baseEdges[index])))
    const inputs = flowInputsFromAttrs(flowMetadata)
    const flow = {
        ...baseFlow,
        schema_version: readStringAttr(flowMetadata, 'schema_version') || String(baseFlow.schema_version ?? '1.0'),
        id: readStringAttr(flowMetadata, 'id') || String(baseFlow.id ?? sanitizeFlowId(flowName)),
        title: readStringAttr(flowMetadata, 'title') || flowName,
        description: readStringAttr(flowMetadata, 'description') || String(baseFlow.description ?? ''),
        goal: readStringAttr(flowMetadata, 'goal') || String(baseFlow.goal ?? ''),
        inputs: inputs.length > 0 ? inputs : (Array.isArray(baseFlow.inputs) ? baseFlow.inputs : []),
        defaults: flowDefaultsFromAttrs(flowMetadata, asRecord(baseFlow.defaults)),
        nodes,
        edges,
        metadata: mergeExtensions(asRecord(baseFlow.metadata), flowMetadata, FLOW_METADATA_CORE_KEYS),
    }
    return `${yamlBlock(flow)}\n`
}
