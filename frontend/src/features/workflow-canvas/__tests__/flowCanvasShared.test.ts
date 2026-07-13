import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'

import { buildHydratedFlowGraph, layoutWithElk } from '@/features/workflow-canvas/flowCanvasShared'
import { filterAuthoredEdges, filterAuthoredNodes } from '@/features/workflow-canvas/derivedPreview'
import { EDGE_RENDER_ROUTE_KEY } from '@/lib/flowLayout'
import type { PreviewResponsePayload } from '@/lib/attractorClient'
import { generateFlowYaml } from '@/lib/flowYamlUtils'
import type { EdgeRoute } from '@/lib/edgeRouting'
import { describe, expect, it } from 'vitest'

const repoRoot = resolve(process.cwd(), '..')
const previewFixtureCache = new Map<string, PreviewResponsePayload>()
const require = createRequire(import.meta.url)
const { load: loadYaml } = require('js-yaml') as { load: (source: string) => unknown }

function loadStarterFlowPreview(flowRelativePath: string, expandChildren: boolean): PreviewResponsePayload {
    const cacheKey = `${flowRelativePath}:${expandChildren ? 'expanded' : 'parent-only'}`
    const cached = previewFixtureCache.get(cacheKey)
    if (cached) {
        return cached
    }

    const flowPath = resolve(repoRoot, flowRelativePath)
    const payload = buildPreviewPayloadFromFlowFile(flowPath, expandChildren)

    previewFixtureCache.set(cacheKey, payload)
    return payload
}

type LooseRecord = Record<string, unknown>

interface LooseFlowNode extends LooseRecord {
    kind?: string
    label?: string
    config?: LooseRecord & { kind?: string; flow_ref?: string; prompt?: string }
    extensions?: LooseRecord
    execution?: LooseRecord
}

interface LooseFlowEdge extends LooseRecord {
    from?: string
    to?: string
    label?: string
    condition?: string
    extensions?: LooseRecord
}

interface LooseFlowDefinition extends LooseRecord {
    title?: string
    goal?: string
    metadata?: LooseRecord
    defaults?: {
        llm_model?: string
        llm_provider?: string
        reasoning_effort?: string
    }
    nodes?: Record<string, LooseFlowNode>
    edges?: LooseFlowEdge[]
}

function buildPreviewPayloadFromFlowFile(flowPath: string, expandChildren: boolean): PreviewResponsePayload {
    if (flowPath.endsWith('.yaml') || flowPath.endsWith('.yml')) {
        return buildPreviewPayloadFromFlowDefinition(flowPath, expandChildren)
    }
    return buildPreviewPayloadFromYamlCompatFile(flowPath, expandChildren)
}

function buildPreviewPayloadFromFlowDefinition(flowPath: string, expandChildren: boolean): PreviewResponsePayload {
    const flow = parseFlowYamlFile(flowPath)
    const graph = flowDefinitionToPreviewGraph(flow)
    if (expandChildren) {
        const childPreviews = Object.entries(flow.nodes ?? {})
            .filter(([, node]) => node.kind === 'subflow' || node.config?.kind === 'subflow')
            .map(([nodeId, node]) => {
                const flowRef = node.config?.flow_ref
                if (typeof flowRef !== 'string' || flowRef.length === 0) {
                    return null
                }
                const childPath = resolve(dirname(flowPath), flowRef)
                const childFlow = parseFlowYamlFile(childPath)
                return [nodeId, {
                    flow_name: flowRef,
                    flow_path: childPath,
                    flow_label: typeof childFlow.title === 'string' && childFlow.title.length > 0 ? childFlow.title : flowRef,
                    provenance: 'derived_child_preview',
                    graph: flowDefinitionToPreviewGraph(childFlow),
                }] as const
            })
            .filter((entry): entry is readonly [string, Record<string, unknown>] => entry !== null)
        if (childPreviews.length > 0) {
            graph.child_previews = Object.fromEntries(childPreviews)
        }
    }
    return {
        status: 'ok',
        flow,
        graph,
    }
}

function parseFlowYamlFile(flowPath: string): LooseFlowDefinition {
    return loadYaml(readFileSync(flowPath, 'utf-8')) as LooseFlowDefinition
}

function flowDefinitionToPreviewGraph(flow: LooseFlowDefinition): NonNullable<PreviewResponsePayload['graph']> {
    const graphAttrs = {
        ...(flow.metadata ?? {}),
        label: flow.title,
        goal: flow.goal,
        llm_model: flow.defaults?.llm_model,
        llm_provider: flow.defaults?.llm_provider,
        reasoning_effort: flow.defaults?.reasoning_effort,
    }
    const nodes = Object.entries(flow.nodes ?? {}).map(([id, typedNode]) => {
        return {
            ...(typedNode.extensions ?? {}),
            ...(typedNode.execution ?? {}),
            id,
            label: typedNode.label,
            kind: typedNode.kind,
            type: typedNode.kind === 'subflow' ? 'stack.manager_loop' : typedNode.kind,
            prompt: typedNode.config?.prompt,
        }
    })
    const edges = Array.isArray(flow.edges)
        ? flow.edges.map((edge) => ({
            ...(edge.extensions ?? {}),
            from: edge.from,
            to: edge.to,
            label: edge.label,
            condition: edge.condition,
        }))
        : []
    return {
        metadata: graphAttrs,
        nodes,
        edges,
    }
}

function buildPreviewPayloadFromYamlCompatFile(flowPath: string, expandChildren: boolean): PreviewResponsePayload {
    const source = readFileSync(flowPath, 'utf-8')
    const graph = parseLineOrientedYamlCompat(source)
    if (expandChildren) {
        const childYamlfile = typeof graph.metadata?.['stack.child_yamlfile'] === 'string'
            ? graph.metadata['stack.child_yamlfile']
            : null
        if (childYamlfile) {
            const childPath = resolve(dirname(flowPath), childYamlfile)
            graph.child_previews = Object.fromEntries(
                graph.nodes
                    .filter((node) => node.type === 'stack.manager_loop' || node.shape === 'house')
                    .map((node) => {
                        const childGraph = parseLineOrientedYamlCompat(readFileSync(childPath, 'utf-8'))
                        return [String(node.id), {
                            flow_name: childYamlfile,
                            flow_path: childPath,
                            flow_label: typeof childGraph.metadata?.label === 'string' ? childGraph.metadata.label : childYamlfile,
                            provenance: 'derived_child_preview',
                            graph: childGraph,
                        }]
                    }),
            )
        }
    }
    return {
        status: 'ok',
        graph,
    }
}

function parseLineOrientedYamlCompat(source: string): NonNullable<PreviewResponsePayload['graph']> {
    const graphAttrs: Record<string, unknown> = {}
    const nodes: Array<Record<string, unknown>> = []
    const edges: Array<Record<string, unknown>> = []

    source.split(/\r?\n/).forEach((rawLine) => {
        const line = rawLine.trim()
        if (!line || line === '{' || line === '}') {
            return
        }
        if (line.startsWith('graph [')) {
            Object.assign(graphAttrs, parseAttrs(extractAttrBody(line)))
            return
        }
        const edgeMatch = line.match(/^"?([^"\s]+)"?\s*->\s*"?([^"\s;[]+)"?\s*(?:\[(.*)\])?;?$/)
        if (edgeMatch) {
            edges.push({
                from: edgeMatch[1],
                to: edgeMatch[2],
                ...parseAttrs(edgeMatch[3] ?? ''),
            })
            return
        }
        const nodeMatch = line.match(/^"?([^"\s[]+)"?\s*\[(.*)\];?$/)
        if (nodeMatch && !['node', 'edge'].includes(nodeMatch[1])) {
            nodes.push({
                id: nodeMatch[1],
                ...parseAttrs(nodeMatch[2]),
            })
        }
    })

    return {
        metadata: graphAttrs,
        nodes,
        edges,
    }
}

function extractAttrBody(statement: string): string {
    const start = statement.indexOf('[')
    const end = statement.lastIndexOf(']')
    return start >= 0 && end > start ? statement.slice(start + 1, end) : ''
}

function parseAttrs(body: string): Record<string, unknown> {
    const attrs: Record<string, unknown> = {}
    splitAttrs(body).forEach((part) => {
        const index = part.indexOf('=')
        if (index <= 0) {
            return
        }
        const key = part.slice(0, index).trim()
        const rawValue = part.slice(index + 1).trim()
        attrs[key] = parseAttrValue(rawValue)
    })
    return attrs
}

function splitAttrs(body: string): string[] {
    const parts: string[] = []
    let current = ''
    let inString = false
    let escaped = false
    for (const character of body) {
        if (escaped) {
            current += character
            escaped = false
            continue
        }
        if (character === '\\') {
            current += character
            escaped = true
            continue
        }
        if (character === '"') {
            current += character
            inString = !inString
            continue
        }
        if (character === ',' && !inString) {
            parts.push(current.trim())
            current = ''
            continue
        }
        current += character
    }
    if (current.trim()) {
        parts.push(current.trim())
    }
    return parts
}

function parseAttrValue(rawValue: string): string | number | boolean {
    if (rawValue.startsWith('"') && rawValue.endsWith('"')) {
        return rawValue.slice(1, -1).replace(/\\"/g, '"').replace(/\\n/g, '\n').replace(/\\\\/g, '\\')
    }
    if (rawValue === 'true') {
        return true
    }
    if (rawValue === 'false') {
        return false
    }
    const numeric = Number(rawValue)
    return Number.isFinite(numeric) && rawValue.trim() !== '' ? numeric : rawValue
}

function getRenderRoute(edge: { data?: Record<string, unknown> | undefined }): EdgeRoute | null {
    const route = edge.data?.[EDGE_RENDER_ROUTE_KEY]
    if (!Array.isArray(route)) {
        return null
    }
    const normalized = route
        .map((point) => {
            if (
                !point
                || typeof point !== 'object'
                || !Number.isFinite((point as { x?: unknown }).x)
                || !Number.isFinite((point as { y?: unknown }).y)
            ) {
                return null
            }
            return {
                x: (point as { x: number }).x,
                y: (point as { y: number }).y,
            }
        })
        .filter((point): point is EdgeRoute[number] => point !== null)

    return normalized.length >= 2 ? normalized : null
}

function summarizeLaidOutGraph(graph: Awaited<ReturnType<typeof layoutWithElk>>) {
    const round = (value: number) => Math.round(value * 100) / 100
    return {
        nodes: [...graph.nodes]
            .sort((left, right) => left.id.localeCompare(right.id))
            .map((node) => ({
                id: node.id,
                x: round(node.position.x),
                y: round(node.position.y),
            })),
        edges: [...graph.edges]
            .sort((left, right) => left.id.localeCompare(right.id))
            .map((edge) => {
                const route = getRenderRoute(edge) ?? []
                return {
                    id: edge.id,
                    source: edge.source,
                    target: edge.target,
                    pointCount: route.length,
                    start: route[0] ? { x: round(route[0].x), y: round(route[0].y) } : null,
                    end: route.at(-1) ? { x: round(route.at(-1)!.x), y: round(route.at(-1)!.y) } : null,
                }
            }),
    }
}

describe('flowCanvasShared', () => {
    it('hydrates mixed-kind graphs with kind-derived node types and dimensions', () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'start', label: 'Start', kind: 'start' },
                    { id: 'human', label: 'Human', kind: 'human_gate' },
                    { id: 'manager', label: 'Manager', kind: 'subflow' },
                    { id: 'custom', label: 'Custom', kind: 'custom' },
                ],
                edges: [],
            },
        }

        const hydrated = buildHydratedFlowGraph('shape-canvas.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        expect(hydrated?.nodes).toMatchObject([
            {
                id: 'start',
                type: 'startNode',
                style: { width: 168, height: 96 },
                data: { shape: 'Mdiamond' },
            },
            {
                id: 'human',
                type: 'humanGateNode',
                style: { width: 228, height: 116 },
                data: { shape: 'hexagon' },
            },
            {
                id: 'manager',
                type: 'managerNode',
                style: { width: 236, height: 124 },
                data: { shape: 'house' },
            },
            {
                id: 'custom',
                type: 'taskNode',
                style: { width: 220, height: 110 },
                data: { shape: 'box' },
            },
        ])
    })

    it('produces routed render polylines with distinct fan-in touch points', async () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'left', label: 'Left', shape: 'box' },
                    { id: 'right', label: 'Right', shape: 'box' },
                    { id: 'join', label: 'Join', shape: 'tripleoctagon' },
                ],
                edges: [
                    { from: 'start', to: 'left' },
                    { from: 'start', to: 'right' },
                    { from: 'left', to: 'join' },
                    { from: 'right', to: 'join' },
                ],
            },
        }

        const hydrated = buildHydratedFlowGraph('routing-canvas.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        const layoutGraph = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])
        expect(layoutGraph.layout.edgeLayouts).not.toEqual({})
        expect(layoutGraph.edges.every((edge) => (getRenderRoute(edge)?.length ?? 0) >= 2)).toBe(true)

        const leftJoinRoute = getRenderRoute(
            layoutGraph.edges.find((edge) => edge.source === 'left' && edge.target === 'join') ?? {},
        )
        const rightJoinRoute = getRenderRoute(
            layoutGraph.edges.find((edge) => edge.source === 'right' && edge.target === 'join') ?? {},
        )

        expect(leftJoinRoute?.at(-1)).not.toEqual(rightJoinRoute?.at(-1))
    })

    it('restores saved node positions and routed edges when saved layout exists', async () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'task', label: 'Task', shape: 'box' },
                    { id: 'exit', label: 'Exit', shape: 'Msquare' },
                ],
                edges: [
                    { from: 'start', to: 'task' },
                    { from: 'task', to: 'exit' },
                ],
            },
        }

        const hydrated = buildHydratedFlowGraph('restore-layout.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        const firstLayout = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])
        const restoredLayout = await layoutWithElk(
            hydrated?.nodes ?? [],
            hydrated?.edges ?? [],
            {
                savedLayout: firstLayout.layout,
            },
        )

        expect(summarizeLaidOutGraph(restoredLayout)).toEqual(summarizeLaidOutGraph(firstLayout))
    })

    it('routes reciprocal edges as distinct polylines', async () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'implement', label: 'Implement', shape: 'box' },
                    { id: 'evaluate', label: 'Evaluate', shape: 'box' },
                ],
                edges: [
                    { from: 'implement', to: 'evaluate' },
                    { from: 'evaluate', to: 'implement', label: 'Fix' },
                ],
            },
        }

        const hydrated = buildHydratedFlowGraph('software-development/implement-change-request.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })

        expect(hydrated).not.toBeNull()
        const layoutGraph = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])
        const forwardRoute = getRenderRoute(
            layoutGraph.edges.find((edge) => edge.source === 'implement' && edge.target === 'evaluate') ?? {},
        )
        const backRoute = getRenderRoute(
            layoutGraph.edges.find((edge) => edge.source === 'evaluate' && edge.target === 'implement') ?? {},
        )

        expect(forwardRoute).not.toBeNull()
        expect(backRoute).not.toBeNull()
        expect(backRoute).not.toEqual(forwardRoute)
    })

    it('builds a namespaced one-level child preview cluster when expansion is enabled', () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'manager', label: 'Manager', shape: 'house', type: 'stack.manager_loop' },
                ],
                edges: [
                    { from: 'start', to: 'manager' },
                ],
                child_previews: {
                    manager: {
                        flow_name: 'child-worker.yaml',
                        flow_path: '/tmp/child-worker.yaml',
                        flow_label: 'Child Worker',
                        read_only: true,
                        provenance: 'derived_child_preview',
                        graph: {
                            metadata: {},
                            nodes: [
                                { id: 'child_start', label: 'Child Start', shape: 'Mdiamond' },
                                { id: 'nested_manager', label: 'Nested Manager', shape: 'house', type: 'stack.manager_loop' },
                            ],
                            edges: [
                                { from: 'child_start', to: 'nested_manager' },
                            ],
                            child_previews: {
                                nested_manager: {
                                    flow_name: 'grandchild.yaml',
                                    flow_path: '/tmp/grandchild.yaml',
                                    flow_label: 'Grandchild',
                                    graph: {
                                        metadata: {},
                                        nodes: [
                                            { id: 'grandchild_start', label: 'Grandchild Start', shape: 'Mdiamond' },
                                        ],
                                        edges: [],
                                    },
                                },
                            },
                        },
                    },
                },
            },
        }

        const hydrated = buildHydratedFlowGraph('parent.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        }, undefined, {
            expandChildren: true,
        })

        expect(hydrated).not.toBeNull()
        expect(hydrated?.nodes.map((node) => node.id)).toEqual(expect.arrayContaining([
            'start',
            'manager',
            '__child_preview_cluster__manager',
            '__child_preview__manager__child_start',
            '__child_preview__manager__nested_manager',
        ]))
        expect(hydrated?.nodes.find((node) => node.id === '__child_preview__manager__child_start')?.selectable).toBe(false)
        expect(hydrated?.nodes.find((node) => node.id === '__child_preview_cluster__manager')?.data).toMatchObject({
            label: 'Child Flow Preview: Child Worker',
        })
        expect(hydrated?.nodes.some((node) => node.id.includes('grandchild'))).toBe(false)
        expect(hydrated?.edges.find((edge) => edge.id === 'e-manager-child-preview-link')).toMatchObject({
            source: 'manager',
            target: '__child_preview__manager__child_start',
        })
    })

    it('hydrates typed FlowDefinition previews, expands children, and round-trips YAML without metadata loss', () => {
        const flow = {
            schema_version: '1.0',
            id: 'typed_parent',
            title: 'Typed Parent',
            description: 'Typed description',
            goal: 'Typed goal',
            inputs: [
                {
                    key: 'context.ticket',
                    label: 'Ticket',
                    type: 'string',
                    description: 'Ticket id',
                    required: true,
                    default: 'SP-1',
                },
            ],
            defaults: {
                max_retries: 2,
                llm_model: 'gpt-5.4',
                llm_provider: 'openai',
                reasoning_effort: 'high',
            },
            nodes: {
                start: {
                    kind: 'start',
                    label: 'Start',
                    config: { kind: 'start' },
                    ui: { x: 10, y: 20 },
                },
                review: {
                    kind: 'human_gate',
                    label: 'Human Review',
                    description: 'Approval point',
                    config: {
                        kind: 'human_gate',
                        prompt: 'Approve?',
                        decisions: [{ label: 'Approve', value: 'approve' }],
                    },
                    retry: { max_retries: 1 },
                    execution: { llm_model: 'gpt-5.4-mini' },
                },
                child: {
                    kind: 'subflow',
                    label: 'Child Flow',
                    config: {
                        kind: 'subflow',
                        flow_ref: 'child.yaml',
                        input_map: { 'context.ticket': 'context.ticket' },
                    },
                },
                done: {
                    kind: 'exit',
                    label: 'Done',
                    config: { kind: 'exit' },
                },
            },
            edges: [
                { from: 'start', to: 'review', label: 'begin', condition: '', weight: 3, transition: 'next' },
                { from: 'review', to: 'child', label: 'approved', condition: 'decision == "approve"', weight: 4 },
                { from: 'child', to: 'done', label: 'complete', condition: '', weight: 5 },
            ],
            metadata: { owner: 'workflow-team' },
        }
        const childFlow = {
            schema_version: '1.0',
            id: 'typed_child',
            title: 'Typed Child',
            nodes: {
                child_start: { kind: 'start', label: 'Child Start', config: { kind: 'start' } },
                child_done: { kind: 'exit', label: 'Child Done', config: { kind: 'exit' } },
            },
            edges: [{ from: 'child_start', to: 'child_done' }],
        }
        const preview: PreviewResponsePayload = {
            status: 'ok',
            flow,
            graph: {
                ...flowDefinitionToPreviewGraph(flow),
                child_previews: {
                    child: {
                        flow_name: 'child.yaml',
                        flow_path: '/tmp/child.yaml',
                        flow_label: 'Typed Child',
                        graph: flowDefinitionToPreviewGraph(childFlow),
                    },
                },
            },
        }

        const hydrated = buildHydratedFlowGraph('typed-parent.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        }, undefined, {
            expandChildren: true,
        })

        expect(hydrated).not.toBeNull()
        expect(hydrated?.nodes.find((node) => node.id === 'start')).toMatchObject({
            type: 'startNode',
            data: { kind: 'start', config: { kind: 'start' } },
        })
        expect(hydrated?.nodes.find((node) => node.id === 'review')).toMatchObject({
            type: 'humanGateNode',
            data: {
                kind: 'human_gate',
                prompt: 'Approve?',
                retry: { max_retries: 1 },
                execution: { llm_model: 'gpt-5.4-mini' },
            },
        })
        expect(hydrated?.nodes.find((node) => node.id === 'child')).toMatchObject({
            type: 'managerNode',
            data: { kind: 'subflow', flow_ref: 'child.yaml' },
        })
        expect(hydrated?.nodes.find((node) => node.id === 'done')).toMatchObject({
            type: 'exitNode',
            data: { kind: 'exit' },
        })
        expect(hydrated?.nodes.some((node) => node.id === '__child_preview__child__child_start')).toBe(true)

        const yaml = generateFlowYaml(
            'typed-parent.yaml',
            filterAuthoredNodes(hydrated?.nodes ?? []),
            filterAuthoredEdges(hydrated?.edges ?? []),
            hydrated?.flowMetadata ?? {},
            {
                flow: hydrated?.flow,
            },
        )
        const roundTripped = loadYaml(yaml) as LooseRecord

        expect(roundTripped).toMatchObject({
            schema_version: '1.0',
            id: 'typed_parent',
            title: 'Typed Parent',
            description: 'Typed description',
            goal: 'Typed goal',
            defaults: {
                max_retries: 2,
                llm_model: 'gpt-5.4',
                llm_provider: 'openai',
                reasoning_effort: 'high',
            },
            metadata: { owner: 'workflow-team' },
        })
        expect(roundTripped.inputs).toEqual(flow.inputs)
        expect(roundTripped.nodes.review).toMatchObject({
            kind: 'human_gate',
            label: 'Human Review',
            description: 'Approval point',
            config: {
                kind: 'human_gate',
                prompt: 'Approve?',
                decisions: [{ label: 'Approve', value: 'approve' }],
            },
            retry: { max_retries: 1 },
            execution: { llm_model: 'gpt-5.4-mini' },
        })
        expect(roundTripped.nodes.child.config).toEqual({
            kind: 'subflow',
            flow_ref: 'child.yaml',
            input_map: { 'context.ticket': 'context.ticket' },
        })
        expect(roundTripped.edges[0]).toMatchObject({
            from: 'start',
            to: 'review',
            label: 'begin',
            weight: 3,
            transition: 'next',
        })
    })

    it('does not materialize implicit node or graph defaults when saving after apply-to-nodes style edits', () => {
        const sourceYaml = `
            schema_version: "1"
            id: implement_spec_program
            title: Implement Spec Program
        `
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {
                    label: 'Implement Spec Program',
                },
                nodes: [
                    { id: 'start', label: 'Start', shape: 'Mdiamond' },
                    { id: 'extract_requirements', label: 'Extract Requirements', shape: 'box', prompt: 'Read spec' },
                ],
                edges: [
                    { from: 'start', to: 'extract_requirements' },
                ],
            },
        }

        const hydrated = buildHydratedFlowGraph('implement-spec.yaml', preview, {
            llm_model: 'gpt-5.4',
            llm_provider: 'openai',
            reasoning_effort: 'high',
        }, sourceYaml)

        expect(hydrated).not.toBeNull()
        expect(hydrated?.graphAttrs.ui_default_llm_model).toBeUndefined()
        expect(hydrated?.nodes[1]?.data).not.toHaveProperty('error_policy')
        expect(hydrated?.nodes[1]?.data).not.toHaveProperty('goal_gate')
        expect(hydrated?.nodes[1]?.data).not.toHaveProperty('auto_status')
        expect(hydrated?.nodes[1]?.data).not.toHaveProperty('allow_partial')

        const updatedNodes = (hydrated?.nodes ?? []).map((node) => ({
            ...node,
            data: {
                ...node.data,
                llm_model: 'gpt-5.4',
                llm_provider: 'openai',
                reasoning_effort: 'high',
            },
        }))
        const yaml = generateFlowYaml('implement-spec.yaml', updatedNodes, hydrated?.edges ?? [], hydrated?.graphAttrs ?? {})

        expect(yaml).toContain('llm_model: gpt-5.4')
        expect(yaml).toContain('llm_provider: openai')
        expect(yaml).toContain('reasoning_effort: high')
        expect(yaml).not.toContain('error_policy: continue')
        expect(yaml).not.toContain('goal_gate: false')
        expect(yaml).not.toContain('auto_status: false')
        expect(yaml).not.toContain('allow_partial: false')
        expect(yaml).not.toContain('ui_default_llm_model')
        expect(yaml).not.toContain('ui_default_llm_provider')
        expect(yaml).not.toContain('ui_default_reasoning_effort')
    })

    it('serializes nested empty collections as parseable YAML', () => {
        const yaml = generateFlowYaml('empty-collections.yaml', [], [], {}, {
            flow: {
                schema_version: '1',
                id: 'empty_collections',
                title: 'Empty Collections',
                nodes: {},
                edges: [],
            },
        })

        expect(loadYaml(yaml)).toMatchObject({
            metadata: {},
            nodes: {},
            edges: [],
        })
    })

    it.each([
        {
            flowName: 'implement-spec.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-spec.yaml',
            expandChildren: false,
            expectedDerivedNodeId: null,
        },
        {
            flowName: 'implement-spec.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-spec.yaml',
            expandChildren: true,
            expectedDerivedNodeId: '__child_preview_cluster__run_milestone',
        },
        {
            flowName: 'implement-milestone.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-milestone.yaml',
            expandChildren: false,
            expectedDerivedNodeId: null,
        },
        {
            flowName: 'implement-milestone.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-milestone.yaml',
            expandChildren: true,
            expectedDerivedNodeId: null,
        },
    ])('keeps ELK placement plus routed geometry stable for $flowName ($expandChildren)', async ({
        flowName,
        flowRelativePath,
        expandChildren,
        expectedDerivedNodeId,
    }) => {
        const sourceYaml = readFileSync(resolve(repoRoot, flowRelativePath), 'utf-8')
        const preview = loadStarterFlowPreview(flowRelativePath, expandChildren)

        expect(preview.status).toBe('ok')
        const hydrated = buildHydratedFlowGraph(
            flowName,
            preview,
            {
                llm_model: '',
                llm_provider: '',
                reasoning_effort: '',
            },
            sourceYaml,
            { expandChildren },
        )

        expect(hydrated).not.toBeNull()
        expect(
            hydrated?.nodes.every((node) =>
                Number.isFinite(node.position.x) && Number.isFinite(node.position.y)),
        ).toBe(true)
        if (expectedDerivedNodeId) {
            expect(hydrated?.nodes.some((node) => node.id === expectedDerivedNodeId)).toBe(true)
        } else {
            expect(hydrated?.nodes.some((node) => node.id.startsWith('__child_preview_'))).toBe(false)
        }

        const firstLayout = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])
        const secondLayout = await layoutWithElk(hydrated?.nodes ?? [], hydrated?.edges ?? [])

        expect(
            firstLayout.nodes.every((node) =>
                Number.isFinite(node.position.x) && Number.isFinite(node.position.y)),
        ).toBe(true)
        expect(firstLayout.edges.every((edge) => (getRenderRoute(edge)?.length ?? 0) >= 2)).toBe(true)
        expect(summarizeLaidOutGraph(secondLayout)).toEqual(summarizeLaidOutGraph(firstLayout))
    })

    it('filters derived child preview nodes and edges out of YAML serialization', () => {
        const preview: PreviewResponsePayload = {
            status: 'ok',
            graph: {
                metadata: {},
                nodes: [
                    { id: 'manager', label: 'Manager', shape: 'house', type: 'stack.manager_loop' },
                ],
                edges: [],
                child_previews: {
                    manager: {
                        flow_name: 'child.yaml',
                        flow_path: '/tmp/child.yaml',
                        flow_label: 'Child',
                        graph: {
                            metadata: {},
                            nodes: [
                                { id: 'child_task', label: 'Child Task', shape: 'box' },
                            ],
                            edges: [],
                        },
                    },
                },
            },
        }

        const hydrated = buildHydratedFlowGraph('parent.yaml', preview, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        }, undefined, {
            expandChildren: true,
        })

        expect(hydrated).not.toBeNull()
        const yaml = generateFlowYaml(
            'parent.yaml',
            filterAuthoredNodes(hydrated?.nodes ?? []),
            filterAuthoredEdges(hydrated?.edges ?? []),
            hydrated?.flowMetadata ?? {},
        )

        expect(yaml).toContain('manager')
        expect(yaml).not.toContain('__child_preview__manager__child_task')
        expect(yaml).not.toContain('Child Flow Preview')
    })
})
