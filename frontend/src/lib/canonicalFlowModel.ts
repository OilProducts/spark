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
    flow: CanonicalFlowDefinition | null
}

export interface CanonicalPreviewGraphPayload {
    nodes: Array<Record<string, unknown>>
    edges: Array<Record<string, unknown>>
    metadata?: Record<string, unknown> | null
    child_previews?: Record<string, unknown> | null
}

export interface CanonicalModelBuildOptions {
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
// Routes are carried by the canvas edges, not by node attrs.
const UNSUPPORTED_NODE_ATTR_KEYS = new Set<string>(['human.default_choice', 'routes'])
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
    'execution_lock',
    'goal',
    'inputs',
    'max_retries',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
    'thinking',
    'thinking_budget_tokens',
    'reasoning_mode',
    'reasoning_summary',

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
    'role',
    'prompt',
    'options',
    'tool.command',
    'flow_ref',
    'input_map',
    'output_map',
    'routes',
    'stack.child_flow_ref',
    'retry_policy',
    'max_retries',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
    'thinking',
    'thinking_budget_tokens',
    'reasoning_mode',
    'reasoning_summary',
    // Flattened runtime/manager/contracts/parallel fields, serialized back into their typed sections.
    'session',
    'class',
    'timeout',
    'spark.reads_context',
    'spark.writes_context',
    'manager.poll_interval',
    'manager.max_cycles',
    'manager.stop_condition',
    'manager.actions',
    'manager.steer_cooldown',
    'stack.child_autostart',
    'max_parallel',
])
const EDGE_EXTENSION_CORE_KEYS = new Set(['kind', 'decision', 'description', 'label', 'extensions'])
const VALID_NODE_KINDS = new Set([
    'start',
    'exit',
    'agent_task',
    'human_gate',
    'switch',
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

// Canvas edges are a view of each node's `routes` map plus each fork's
// structural branch and join connections, in the server's preview order.
function flowEdgesFromDefinition(flow: CanonicalFlowDefinition): CanonicalFlowEdge[] {
    return Object.entries(asRecord(flow.nodes) ?? {}).flatMap(([source, nodeValue]) => {
        const node = asRecord(nodeValue)
        const routeEdges: CanonicalFlowEdge[] = Object.entries(asRecord(node?.routes) ?? {}).flatMap(([decision, routeValue]) => {
            const route = asRecord(routeValue)
            if (typeof route?.to !== 'string') return []
            return [{
                source,
                target: route.to,
                attrs: {
                    ...cloneCanonicalAttrMap(route.extensions),
                    kind: 'route',
                    decision,
                    description: typeof route.description === 'string' ? route.description : '',
                    label: typeof route.label === 'string' ? route.label : '',
                },
            }]
        })
        const config = asRecord(node?.config)
        if (node?.kind !== 'parallel' || !config) return routeEdges
        const branches = Array.isArray(config.branches)
            ? config.branches.filter((branch): branch is string => typeof branch === 'string')
            : []
        return [
            ...routeEdges,
            ...branches.map((target) => ({ source, target, attrs: { kind: 'branch' } })),
            ...(typeof config.join === 'string' && config.join ? [{ source, target: config.join, attrs: { kind: 'join' } }] : []),
        ]
    })
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
            if (asRecord(config.output_map)) {
                attrs.output_map = cloneCanonicalAttrMap(config.output_map)
            }
        }
        if (kind === 'parallel' && typeof config.max_parallel === 'number') {
            attrs.max_parallel = config.max_parallel
        }
    }

    const execution = asRecord(attrs.execution)
    if (execution) {
        ;(['llm_model', 'llm_provider', 'llm_profile', 'reasoning_effort', 'thinking', 'thinking_budget_tokens', 'reasoning_mode', 'reasoning_summary'] as const).forEach((key) => {
            if (typeof execution[key] === 'string' || typeof execution[key] === 'number') {
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
        ;(['session', 'class', 'timeout'] as const).forEach((key) => {
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
    const executionLock = asRecord(flow?.execution_lock)
    if (typeof executionLock?.key === 'string') metadata.execution_lock = executionLock.key
    if (typeof flow?.goal === 'string') metadata.goal = flow.goal
    if (Array.isArray(flow?.inputs)) metadata.inputs = JSON.stringify(flow.inputs)
    if (defaults) {
        if (typeof defaults.max_retries === 'number') metadata.max_retries = defaults.max_retries
        if (typeof defaults.llm_model === 'string') metadata.llm_model = defaults.llm_model
        if (typeof defaults.llm_provider === 'string') metadata.llm_provider = defaults.llm_provider
        if (typeof defaults.llm_profile === 'string') metadata.llm_profile = defaults.llm_profile
        if (typeof defaults.reasoning_effort === 'string') metadata.reasoning_effort = defaults.reasoning_effort
        if (defaults.thinking != null) metadata.thinking = String(defaults.thinking)
        if (defaults.thinking_budget_tokens != null) metadata.thinking_budget_tokens = String(defaults.thinking_budget_tokens)
        if (defaults.reasoning_mode != null) metadata.reasoning_mode = String(defaults.reasoning_mode)
        if (defaults.reasoning_summary != null) metadata.reasoning_summary = String(defaults.reasoning_summary)

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

    const edges: CanonicalFlowEdge[] = baseFlow && asRecord(baseFlow.nodes) ? flowEdgesFromDefinition(baseFlow) : graph.edges.flatMap((edgePayload) => {
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
        flowMetadata: canonicalizeFlowMetadata(flowMetadataFromFlow(baseFlow, graph.metadata)),
        nodes,
        edges,
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

export const DEFAULT_EXIT_RESULT = { name: 'completed', outcome: 'success' }

// The node's config: its editor config (`attrs.config`, else the stored one)
// with the flattened editor fields applied. Fields the server requires stay
// present even when empty, so an incomplete node reaches validation.
function nodeConfigFromAttrs(
    kind: string,
    attrs: CanonicalAttrMap,
    baseConfig: Record<string, unknown> | null | undefined,
    fork: { branches: string[]; join: string | undefined },
): Record<string, unknown> {
    const base = { ...(asRecord(attrs.config) ?? baseConfig ?? {}) }
    if (kind === 'agent_task' || kind === 'human_gate') {
        return compactObject({ ...base, kind, prompt: readStringAttr(attrs, 'prompt') })
    }
    if (kind === 'tool') return compactObject({ ...base, kind, command: readStringAttr(attrs, 'tool.command') })
    if (kind === 'subflow') {
        return {
            ...compactObject({
                ...base,
                kind,
                input_map: asRecord(attrs.input_map) ?? asRecord(base.input_map) ?? undefined,
                output_map: asRecord(attrs.output_map) ?? asRecord(base.output_map) ?? undefined,
            }),
            flow_ref: readStringAttr(attrs, 'flow_ref') || String(base.flow_ref ?? 'child'),
            results: asRecord(base.results) ?? {},
        }
    }
    if (kind === 'parallel') {
        return {
            ...compactObject({
                ...base,
                kind,
                max_parallel: readNumberAttr(attrs, 'max_parallel') ?? base.max_parallel,
            }),
            branches: fork.branches,
            join: fork.join ?? (typeof base.join === 'string' ? base.join : ''),
        }
    }
    if (kind === 'switch') {
        return {
            ...base,
            kind,
            on: typeof base.on === 'string' ? base.on : '',
            cases: asRecord(base.cases) ?? {},
            otherwise: typeof base.otherwise === 'string' ? base.otherwise : '',
        }
    }
    if (kind === 'exit') {
        return { ...compactObject({ ...base, kind }), result: asRecord(base.result) ?? { ...DEFAULT_EXIT_RESULT } }
    }
    return compactObject({ ...base, kind })
}

const DUPLICATE_DECISION_MARKER = ' (duplicate '

function duplicateDecisionKey(decision: string, count: number): string {
    return `${decision}${DUPLICATE_DECISION_MARKER}${count})`
}

/** Decision IDs a flow declares more than once on one node, as "node.decision". */
export function duplicateDecisionsInFlow(flow: unknown): string[] {
    const nodes = asRecord(asRecord(flow)?.nodes) ?? {}
    return Object.entries(nodes).flatMap(([nodeId, node]) => (
        Object.keys(asRecord(asRecord(node)?.routes) ?? {})
            .filter((key) => key.includes(DUPLICATE_DECISION_MARKER))
            .map((key) => `${nodeId}.${key.slice(0, key.indexOf(DUPLICATE_DECISION_MARKER))}`)
    ))
}

// Edges leaving a node, serialized back into that node's own `routes` map
// (or, for a fork, its branches and join). A drawn edge without a decision ID
// gets a free `next`-style ID; validation then asks for its description.
function nodeRoutingFromEdges(kind: string, edges: CanonicalFlowEdge[]) {
    if (kind === 'parallel') {
        const joinEdge = edges.find((edge) => edge.attrs.kind === 'join')
        return {
            routes: undefined,
            fork: {
                branches: edges.filter((edge) => edge !== joinEdge).map((edge) => edge.target),
                join: joinEdge?.target,
            },
        }
    }
    // A Map, so decision IDs such as `constructor` are never confused with
    // inherited object properties.
    const routes = new Map<string, Record<string, unknown>>()
    const taken = new Set(edges.map((edge) => readStringAttr(edge.attrs, 'decision')).filter(Boolean))
    const seen = new Map<string, number>()
    edges.forEach((edge) => {
        let decision = readStringAttr(edge.attrs, 'decision')
        if (decision && routes.has(decision)) {
            // Two edges declaring one decision are an authoring error: keep
            // both under an ID that can never validate, so preview reports
            // it and save refuses it, instead of renaming either.
            const count = (seen.get(decision) ?? 1) + 1
            seen.set(decision, count)
            decision = duplicateDecisionKey(decision, count)
        } else if (!decision) {
            let suffix = 1
            do {
                decision = suffix === 1 ? 'next' : `next_${suffix}`
                suffix += 1
            } while (taken.has(decision) || routes.has(decision))
        }
        routes.set(decision, compactObject({
            to: edge.target,
            description: readStringAttr(edge.attrs, 'description'),
            label: readStringAttr(edge.attrs, 'label') || undefined,
            extensions: extensionAttrs(edge.attrs, EDGE_EXTENSION_CORE_KEYS),
        }))
    })
    return {
        routes: routes.size > 0 ? Object.fromEntries(routes) : undefined,
        fork: { branches: [], join: undefined },
    }
}

function extensionAttrs(attrs: CanonicalAttrMap, coreKeys: Set<string>): CanonicalAttrMap {
    return Object.fromEntries(
        Object.entries(attrs).filter(([key, value]) => (
            !coreKeys.has(key)
            // Editor form fields default to '' when unset; they are not extensions.
            && value !== ''
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
        llm_model: readStringAttr(attrs, 'llm_model') || baseDefaults?.llm_model,
        llm_provider: readStringAttr(attrs, 'llm_provider') || baseDefaults?.llm_provider,
        llm_profile: readStringAttr(attrs, 'llm_profile') || baseDefaults?.llm_profile,
        reasoning_effort: readStringAttr(attrs, 'reasoning_effort') || baseDefaults?.reasoning_effort,
        thinking: 'thinking' in attrs ? readStringAttr(attrs, 'thinking') || undefined : baseDefaults?.thinking,
        thinking_budget_tokens: 'thinking_budget_tokens' in attrs ? attrs.thinking_budget_tokens == null || attrs.thinking_budget_tokens === '' ? undefined : Number(attrs.thinking_budget_tokens) : baseDefaults?.thinking_budget_tokens,
        reasoning_mode: 'reasoning_mode' in attrs ? readStringAttr(attrs, 'reasoning_mode') || undefined : baseDefaults?.reasoning_mode,
        reasoning_summary: 'reasoning_summary' in attrs ? readStringAttr(attrs, 'reasoning_summary') || undefined : baseDefaults?.reasoning_summary,

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
        thinking: 'thinking' in attrs ? readStringAttr(attrs, 'thinking') || undefined : baseExecution?.thinking,
        thinking_budget_tokens: 'thinking_budget_tokens' in attrs ? attrs.thinking_budget_tokens == null || attrs.thinking_budget_tokens === '' ? undefined : Number(attrs.thinking_budget_tokens) : baseExecution?.thinking_budget_tokens,
        reasoning_mode: 'reasoning_mode' in attrs ? readStringAttr(attrs, 'reasoning_mode') || undefined : baseExecution?.reasoning_mode,
        reasoning_summary: 'reasoning_summary' in attrs ? readStringAttr(attrs, 'reasoning_summary') || undefined : baseExecution?.reasoning_summary,

    })
}

function runtimeFromAttrs(attrs: CanonicalAttrMap, baseRuntime?: Record<string, unknown> | null): Record<string, unknown> | undefined {
    return compactObject({
        ...(baseRuntime ?? {}),
        session: 'session' in attrs ? readStringAttr(attrs, 'session') || undefined : baseRuntime?.session,
        class: readStringAttr(attrs, 'class') || baseRuntime?.class,
        timeout: readStringAttr(attrs, 'timeout') || baseRuntime?.timeout,
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
    baseNode: Record<string, unknown> | null | undefined,
    outgoingEdges: CanonicalFlowEdge[],
): Record<string, unknown> {
    const kind = nodeKindFromAttrs(attrs, baseNode)
    const { routes, fork } = nodeRoutingFromEdges(kind, outgoingEdges)
    const nextNode = compactObject({
        ...(baseNode ?? {}),
        kind,
        label: readStringAttr(attrs, 'label') || String(baseNode?.label ?? nodeId),
        description: readStringAttr(attrs, 'description') || baseNode?.description,
        role: 'role' in attrs ? readStringAttr(attrs, 'role') || undefined : baseNode?.role,
        config: nodeConfigFromAttrs(kind, attrs, asRecord(baseNode?.config), fork),
        context: asRecord(attrs.context) ?? asRecord(baseNode?.context) ?? undefined,
        contracts: contractsFromAttrs(attrs, asRecord(baseNode?.contracts)),
        runtime: runtimeFromAttrs(attrs, asRecord(baseNode?.runtime)),
        manager: managerFromAttrs(attrs, asRecord(baseNode?.manager)),
        retry: retryFromAttrs(attrs, asRecord(baseNode?.retry)),
        execution: executionFromAttrs(attrs, asRecord(baseNode?.execution)),
        ui: asRecord(attrs.ui) ?? asRecord(baseNode?.ui) ?? undefined,
        extensions: mergeExtensions(asRecord(baseNode?.extensions), attrs, NODE_EXTENSION_CORE_KEYS),
        routes,
    })
    return nextNode
}

// The editor edits only the lock key; scope and conflict policy have one allowed value each.
function executionLockFromAttrs(attrs: CanonicalAttrMap, baseLock: unknown): unknown {
    if (!('execution_lock' in attrs)) return baseLock ?? undefined
    const key = readStringAttr(attrs, 'execution_lock').trim()
    return key ? { scope: 'project', key, conflict_policy: 'queue' } : undefined
}

export function buildFlowDefinitionFromCanonicalFlowModel(flowName: string, model: CanonicalFlowModel): CanonicalFlowDefinition {
    const flowMetadata = model.flowMetadata
    const baseFlow = cloneFlowDefinition(model.flow) ?? {}
    const baseNodes = asRecord(baseFlow.nodes)
    delete baseFlow.edges
    const nodes = Object.fromEntries(model.nodes.map((node) => {
        const outgoingEdges = model.edges.filter((edge) => edge.source === node.id)
        return [node.id, nodeFromCanonicalAttrs(node.id, node.attrs, asRecord(baseNodes?.[node.id]), outgoingEdges)]
    }))
    const inputs = flowInputsFromAttrs(flowMetadata)
    return {
        ...baseFlow,
        schema_version: '2',
        id: readStringAttr(flowMetadata, 'id') || String(baseFlow.id ?? sanitizeFlowId(flowName)),
        title: readStringAttr(flowMetadata, 'title') || flowName,
        description: readStringAttr(flowMetadata, 'description') || String(baseFlow.description ?? ''),
        goal: readStringAttr(flowMetadata, 'goal') || String(baseFlow.goal ?? ''),
        execution_lock: executionLockFromAttrs(flowMetadata, baseFlow.execution_lock),
        inputs: inputs.length > 0 ? inputs : (Array.isArray(baseFlow.inputs) ? baseFlow.inputs : []),
        defaults: flowDefaultsFromAttrs(flowMetadata, asRecord(baseFlow.defaults)),
        nodes,
        metadata: mergeExtensions(asRecord(baseFlow.metadata), flowMetadata, FLOW_METADATA_CORE_KEYS),
    }
}
