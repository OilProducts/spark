import { readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'

import { buildHydratedFlowGraph, layoutWithElk } from '@/features/workflow-canvas/flowCanvasShared'
import { filterAuthoredEdges, filterAuthoredNodes } from '@/features/workflow-canvas/derivedPreview'
import { EDGE_RENDER_ROUTE_KEY } from '@/lib/flowLayout'
import type { PreviewResponsePayload } from '@/lib/attractorClient'
import {
    buildCanonicalFlowModelFromEditorState,
    buildFlowDefinitionFromCanonicalFlowModel,
    type CanonicalEditorStateInput,
} from '@/lib/canonicalFlowModel'
import type { EdgeRoute } from '@/lib/edgeRouting'
import { formatDecisionMap, parseDecisionMap } from '@/lib/flowContracts'
import { describe, expect, it } from 'vitest'

const repoRoot = resolve(process.cwd(), '..')
const previewFixtureCache = new Map<string, PreviewResponsePayload>()
const require = createRequire(import.meta.url)
// Frontmatter in flow files is YAML.
const { load: loadFrontmatter } = require('js-yaml') as {
    load: (source: string) => unknown
}

function loadStarterFlowPreview(flowRelativePath: string, expandChildren: boolean): PreviewResponsePayload {
    const cacheKey = `${flowRelativePath}:${expandChildren ? 'expanded' : 'parent-only'}`
    const cached = previewFixtureCache.get(cacheKey)
    if (cached) {
        return cached
    }

    const flowPath = resolve(repoRoot, flowRelativePath)
    const payload = buildPreviewPayloadFromFlowDefinition(flowPath, expandChildren)

    previewFixtureCache.set(cacheKey, payload)
    return payload
}

type LooseRecord = Record<string, unknown>

interface LooseFlowNode extends LooseRecord {
    kind?: string
    label?: string
    config?: LooseRecord & { kind?: string; flow_ref?: string; prompt?: string; branches?: string[]; join?: string }
    extensions?: LooseRecord
    execution?: LooseRecord
    routes?: Record<string, LooseFlowRoute>
}

interface LooseFlowRoute extends LooseRecord {
    to: string
    description?: string
    label?: string
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
}

// The FlowDefinition the editor saves for this canvas state.
function buildFlow(
    flowName: string,
    nodes: CanonicalEditorStateInput['nodes'],
    edges: CanonicalEditorStateInput['edges'],
    flowMetadata: CanonicalEditorStateInput['flowMetadata'],
    flow: CanonicalEditorStateInput['flow'] = null,
) {
    return buildFlowDefinitionFromCanonicalFlowModel(
        flowName,
        buildCanonicalFlowModelFromEditorState(flowName, { nodes, edges, flowMetadata, flow }),
    ) as LooseFlowDefinition & { nodes: Record<string, LooseFlowNode>; inputs?: unknown }
}

function buildPreviewPayloadFromFlowDefinition(flowPath: string, expandChildren: boolean): PreviewResponsePayload {
    const flow = readFlowDirectory(flowPath)
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
                const childFlow = readFlowDirectory(childPath)
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

const NODE_BODY_KEYS: Record<string, string> = {
    agent_task: 'prompt',
    human_gate: 'prompt',
    exit: 'result_summary_prompt',
    tool: 'command',
}

// Assembles a flow directory (flow.md plus one <node>.md per node) the way
// the server loader does.
function readFlowDirectory(flowPath: string): LooseFlowDefinition {
    const readMarkdown = (file: string) => {
        const match = /^---\n([\s\S]*?)\n---\n?([\s\S]*)$/.exec(readFileSync(resolve(flowPath, file), 'utf-8'))
        return {
            meta: (loadFrontmatter(match?.[1] ?? '') ?? {}) as LooseRecord,
            body: (match?.[2] ?? '').trim(),
        }
    }
    const header = readMarkdown('flow.md')
    const nodes: Record<string, LooseFlowNode> = {}
    for (const file of readdirSync(flowPath).filter((name) => name.endsWith('.md') && name !== 'flow.md')) {
        const nodeId = file.slice(0, -'.md'.length)
        const { meta, body } = readMarkdown(file)
        const { type, title, routes, config, ...rest } = meta
        const kind = String(type)
        const bodyKey = NODE_BODY_KEYS[kind]
        const bodyValue = kind === 'tool' ? body.replace(/^`{3,}.*\n|`{3,}$/g, '') : body
        nodes[nodeId] = {
            ...rest,
            kind,
            label: typeof title === 'string' ? title : '',
            config: { ...(config as LooseRecord | undefined), kind, ...(bodyKey && bodyValue ? { [bodyKey]: bodyValue } : {}) },
            ...(routes ? { routes: routes as Record<string, LooseFlowRoute> } : {}),
        }
    }
    return { ...header.meta, goal: header.body, nodes }
}

// The server's derived preview edges: every route, then a fork's branches and join.
function previewEdges(flow: LooseFlowDefinition) {
    return Object.entries(flow.nodes ?? {}).flatMap(([from, node]) => [
        ...Object.entries(node.routes ?? {}).map(([decision, route]) => ({
            source: from, target: route.to, from, to: route.to, kind: 'route', decision,
            description: route.description ?? '', label: route.label ?? '',
        })),
        ...(node.kind === 'parallel'
            ? [
                ...(node.config?.branches ?? []).map((to) => ({ source: from, target: to, from, to, kind: 'branch', decision: null, description: '', label: '' })),
                { source: from, target: node.config?.join, from, to: node.config?.join, kind: 'join', decision: null, description: '', label: '' },
            ]
            : []),
    ])
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
    return {
        metadata: graphAttrs,
        nodes,
        edges: previewEdges(flow),
    }
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
        }, {
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

    it('hydrates typed FlowDefinition previews, expands children, and round-trips the FlowDefinition without metadata loss', () => {
        const flow = {
            schema_version: '2',
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
                    routes: { next: { to: 'review', description: 'Enter the first workflow step.', extensions: { lane: 'main' } } },
                },
                review: {
                    kind: 'human_gate',
                    label: 'Human Review',
                    description: 'Approval point',
                    config: {
                        kind: 'human_gate',
                        prompt: 'Approve?',
                    },
                    retry: { max_retries: 1 },
                    execution: { llm_model: 'gpt-5.4-mini' },
                    routes: {
                        approve: { to: 'child', label: 'Approve', description: 'The proposal is approved.' },
                        reject: { to: 'done', label: 'Reject', description: 'The proposal is rejected.' },
                    },
                },
                child: {
                    kind: 'subflow',
                    label: 'Child Flow',
                    config: {
                        kind: 'subflow',
                        flow_ref: 'child.yaml',
                        input_map: { 'context.ticket': 'context.ticket' },
                        output_map: { 'context.verdict': 'context.child_verdict' },
                        results: { completed: 'child_completed' },
                    },
                    routes: { child_completed: { to: 'done', description: 'The child flow completed.' } },
                },
                done: {
                    kind: 'exit',
                    label: 'Done',
                    config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } },
                },
            },
            metadata: { owner: 'workflow-team' },
        }
        const childFlow = {
            schema_version: '2',
            id: 'typed_child',
            title: 'Typed Child',
            nodes: {
                child_start: {
                    kind: 'start',
                    label: 'Child Start',
                    config: { kind: 'start' },
                    routes: { next: { to: 'child_done', description: 'Enter the first workflow step.' } },
                },
                child_done: { kind: 'exit', label: 'Child Done', config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } } },
            },
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
        }, {
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

        const roundTripped = buildFlow(
            'typed-parent.yaml',
            filterAuthoredNodes(hydrated?.nodes ?? []),
            filterAuthoredEdges(hydrated?.edges ?? []),
            hydrated?.flowMetadata ?? {},
            hydrated?.flow,
        )

        expect(roundTripped).toMatchObject({
            schema_version: '2',
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
            },
            retry: { max_retries: 1 },
            execution: { llm_model: 'gpt-5.4-mini' },
        })
        expect(roundTripped.nodes.child.config).toEqual(flow.nodes.child.config)
        for (const [nodeId, node] of Object.entries(flow.nodes)) {
            expect(roundTripped.nodes[nodeId].routes).toEqual((node as LooseFlowNode).routes)
        }
        expect(roundTripped.nodes.done.config).toEqual(flow.nodes.done.config)
        expect(roundTripped).not.toHaveProperty('edges')
    })

    it('keeps role a node field so editing one role-bearing node leaves the others unchanged', () => {
        const done = { done: { to: 'done', description: 'Reviewed.' } }
        const flow = {
            schema_version: '2',
            id: 'roles',
            title: 'Roles',
            nodes: {
                start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'first', description: 'Enter the first workflow step.' } } },
                first: { kind: 'agent_task', label: 'First', role: 'reviewer', config: { kind: 'agent_task', prompt: 'Review A.' }, routes: { done: { to: 'second', description: 'Reviewed.' } } },
                second: { kind: 'agent_task', label: 'Second', role: 'reviewer', config: { kind: 'agent_task', prompt: 'Review B.' }, routes: done },
                done: { kind: 'exit', label: 'Done', config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } } },
            },
        }
        const hydrated = buildHydratedFlowGraph('roles', { status: 'ok', flow, graph: flowDefinitionToPreviewGraph(flow) }, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })
        const edited = (hydrated?.nodes ?? []).map((node) => (
            node.id === 'first' ? { ...node, data: { ...node.data, prompt: 'Review A twice.' } } : node
        ))
        const saved = buildFlow('roles', edited, hydrated?.edges ?? [], hydrated?.flowMetadata ?? {}, hydrated?.flow)

        expect(saved.nodes.second).toEqual(flow.nodes.second)
        expect(saved.nodes.done).toEqual(flow.nodes.done)
        expect(saved.nodes.first).toEqual({ ...flow.nodes.first, config: { kind: 'agent_task', prompt: 'Review A twice.' } })
    })

    it('saves a cleared Session as absent while preserving or renaming it otherwise', () => {
        const flow = {
            schema_version: '2',
            id: 'sessions',
            title: 'Sessions',
            nodes: {
                start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'work', description: 'Enter the first workflow step.' } } },
                work: { kind: 'agent_task', label: 'Work', runtime: { session: 'implementer' }, config: { kind: 'agent_task', prompt: 'Work.' }, routes: { done: { to: 'done', description: 'Worked.' } } },
                done: { kind: 'exit', label: 'Done', config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } } },
            },
        }
        const saveWith = (source: typeof flow, edit: (data: Record<string, unknown>) => Record<string, unknown>) => {
            const hydrated = buildHydratedFlowGraph('sessions', { status: 'ok', flow: source, graph: flowDefinitionToPreviewGraph(source) }, {
                llm_model: '',
                llm_provider: '',
                reasoning_effort: '',
            })
            const edited = (hydrated?.nodes ?? []).map((node) => (
                node.id === 'work' ? { ...node, data: edit({ ...node.data }) } : node
            ))
            return buildFlow('sessions', edited, hydrated?.edges ?? [], hydrated?.flowMetadata ?? {}, hydrated?.flow)
        }

        const preserved = saveWith(flow, (data) => data)
        expect(preserved.nodes.work.runtime).toEqual({ session: 'implementer' })
        const withoutAttr = saveWith(flow, (data) => {
            delete data.session
            return data
        })
        expect(withoutAttr.nodes.work.runtime).toEqual({ session: 'implementer' })

        const renamed = saveWith(flow, (data) => ({ ...data, session: 'reviewer' }))
        expect(renamed.nodes.work.runtime).toEqual({ session: 'reviewer' })

        const cleared = saveWith(flow, (data) => ({ ...data, session: '' }))
        expect(cleared.nodes.work.runtime?.session).toBeUndefined()
        const reloaded = saveWith(cleared as typeof flow, (data) => data)
        expect(reloaded.nodes.work.runtime?.session).toBeUndefined()
    })

    it('does not materialize implicit node or graph defaults when saving after apply-to-nodes style edits', () => {
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
        })

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
        const saved = JSON.stringify(buildFlow('implement-spec.yaml', updatedNodes, hydrated?.edges ?? [], hydrated?.graphAttrs ?? {}))

        expect(saved).toContain('"llm_model":"gpt-5.4"')
        expect(saved).toContain('"llm_provider":"openai"')
        expect(saved).toContain('"reasoning_effort":"high"')
        expect(saved).not.toContain('"error_policy"')
        expect(saved).not.toContain('"goal_gate"')
        expect(saved).not.toContain('"auto_status"')
        expect(saved).not.toContain('"allow_partial"')
        expect(saved).not.toContain('ui_default_llm_model')
        expect(saved).not.toContain('ui_default_llm_provider')
        expect(saved).not.toContain('ui_default_reasoning_effort')
    })

    it.each([
        {
            flowName: 'implement-spec.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-spec',
            expandChildren: false,
            expectedDerivedNodeId: null,
        },
        {
            flowName: 'implement-spec.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-spec',
            expandChildren: true,
            expectedDerivedNodeId: '__child_preview_cluster__run_milestone',
        },
        {
            flowName: 'implement-milestone.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-milestone',
            expandChildren: false,
            expectedDerivedNodeId: null,
        },
        {
            flowName: 'implement-milestone.yaml',
            flowRelativePath: 'crates/spark-assets/assets/flows/software-development/spec-implementation/implement-milestone',
            expandChildren: true,
            expectedDerivedNodeId: null,
        },
    ])('keeps ELK placement plus routed geometry stable for $flowName ($expandChildren)', async ({
        flowName,
        flowRelativePath,
        expandChildren,
        expectedDerivedNodeId,
    }) => {
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

    it('filters derived child preview nodes and edges out of the saved flow', () => {
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
        }, {
            expandChildren: true,
        })

        expect(hydrated).not.toBeNull()
        const saved = buildFlow(
            'parent.yaml',
            filterAuthoredNodes(hydrated?.nodes ?? []),
            filterAuthoredEdges(hydrated?.edges ?? []),
            hydrated?.flowMetadata ?? {},
        )

        expect(Object.keys(saved.nodes)).toEqual(['manager'])
        expect(JSON.stringify(saved)).not.toContain('__child_preview__manager__child_task')
        expect(JSON.stringify(saved)).not.toContain('Child Flow Preview')
    })
})

it('serializes and clears reasoning controls in node execution and graph defaults', () => {
    const controls = { thinking: 'budget', thinking_budget_tokens: '2048', reasoning_mode: 'pro', reasoning_summary: 'detailed' }
    const nodes = [{ id: 'work', position: { x: 0, y: 0 }, data: { shape: 'box', prompt: 'hello', ...controls } }]
    const flow = buildFlow('reasoning.yaml', nodes, [], controls)
    expect(flow.defaults).toMatchObject({ ...controls, thinking_budget_tokens: 2048 })
    expect(flow.nodes.work.execution).toMatchObject({ ...controls, thinking_budget_tokens: 2048 })
    const cleared = { thinking: '', thinking_budget_tokens: '', reasoning_mode: '', reasoning_summary: '' }
    const updated = JSON.stringify(buildFlow('reasoning.yaml', [{ ...nodes[0], data: { ...nodes[0].data, ...cleared } }], [], cleared, flow))
    for (const field of Object.keys(controls)) expect(updated).not.toContain(`"${field}"`)
})

describe('schema-2 routes and fork structure', () => {
    const hydrate = (flow: LooseFlowDefinition) => {
        const hydrated = buildHydratedFlowGraph('routes', { status: 'ok', flow, graph: flowDefinitionToPreviewGraph(flow) }, {
            llm_model: '',
            llm_provider: '',
            reasoning_effort: '',
        })
        if (!hydrated) throw new Error('expected a hydrated graph')
        return hydrated
    }
    const exitNode = (name: string, outcome: string) => ({ kind: 'exit', label: name, config: { kind: 'exit', result: { name, outcome } } })
    const reviewFlow = {
        schema_version: '2',
        id: 'review_loop',
        title: 'Review loop',
        nodes: {
            start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'review', description: 'Enter the first workflow step.' } } },
            review: {
                kind: 'human_gate',
                label: 'Review',
                config: { kind: 'human_gate', prompt: 'Ship it?' },
                routes: {
                    approve: { to: 'done', label: 'Approve', description: 'Approved for shipping.' },
                    waive: { to: 'done', label: 'Waive', description: 'Shipped without review.' },
                    reject: { to: 'failed', label: 'Reject', description: 'Rejected.' },
                },
            },
            done: exitNode('completed', 'success'),
            failed: exitNode('blocked', 'failure'),
        },
    }

    it('derives one edge per decision, including two decisions to the same node, and saves them back into the routes map', () => {
        const hydrated = hydrate(reviewFlow)
        const reviewEdges = hydrated.edges.filter((edge) => edge.source === 'review')
        expect(reviewEdges.map((edge) => [edge.target, edge.data?.decision, edge.data?.label])).toEqual([
            ['done', 'approve', 'Approve'],
            ['done', 'waive', 'Waive'],
            ['failed', 'reject', 'Reject'],
        ])
        expect(new Set(reviewEdges.map((edge) => edge.id)).size).toBe(3)

        const saved = buildFlow('routes', hydrated.nodes, hydrated.edges, hydrated.flowMetadata, hydrated.flow)
        expect(saved.nodes).toEqual(reviewFlow.nodes)
        expect(saved.schema_version).toBe('2')
        expect(saved).not.toHaveProperty('edges')
        expect(JSON.stringify(saved)).not.toMatch(/"(condition|weight|transition|loop_restart)"/)

        const edited = hydrated.edges.map((edge) => (
            edge.source === 'review' && edge.data?.decision === 'waive'
                ? { ...edge, target: 'failed', data: { ...edge.data, decision: 'defer', description: 'Deferred.', label: 'Defer' } }
                : edge
        ))
        const drawn = { id: 'new-edge', source: 'done', target: 'failed' }
        const resaved = buildFlow('routes', hydrated.nodes, [...edited, drawn], hydrated.flowMetadata, hydrated.flow)
        expect(resaved.nodes.review.routes).toEqual({
            approve: reviewFlow.nodes.review.routes.approve,
            defer: { to: 'failed', label: 'Defer', description: 'Deferred.' },
            reject: reviewFlow.nodes.review.routes.reject,
        })
        // A drawn edge gets a free decision ID and an empty description for validation to flag.
        expect(resaved.nodes.done.routes).toEqual({ next: { to: 'failed', description: '' } })
    })

    it('round-trips switch configuration and case keys with their JSON scalar types', () => {
        const switchFlow = {
            schema_version: '2',
            id: 'switching',
            title: 'Switching',
            nodes: {
                start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'route', description: 'Enter the first workflow step.' } } },
                route: {
                    kind: 'switch',
                    label: 'Route',
                    contracts: { reads_context: ['context.count'] },
                    config: { kind: 'switch', on: 'context.count', cases: { '0': 'zero', '"0"': 'text_zero', 'true': 'zero' }, otherwise: 'other' },
                    routes: {
                        other: { to: 'failed', description: 'Any other value.' },
                        text_zero: { to: 'done', description: 'The string "0".' },
                        zero: { to: 'done', description: 'Number zero or true.' },
                    },
                },
                done: exitNode('completed', 'success'),
                failed: exitNode('blocked', 'failure'),
            },
        }
        const hydrated = hydrate(switchFlow)
        expect(hydrated.nodes.find((node) => node.id === 'route')?.type).toBe('switchNode')
        expect(hydrated.edges.filter((edge) => edge.source === 'route').map((edge) => edge.data?.decision)).toEqual(['other', 'text_zero', 'zero'])

        const saved = buildFlow('switching', hydrated.nodes, hydrated.edges, hydrated.flowMetadata, hydrated.flow)
        expect(saved.nodes.route).toEqual(switchFlow.nodes.route)
        expect(Object.keys(saved.nodes.route.config?.cases as object)).toEqual(['0', '"0"', 'true'])

        const text = formatDecisionMap(switchFlow.nodes.route.config.cases)
        expect(text).toBe('0 = zero\n"0" = text_zero\ntrue = zero')
        expect(parseDecisionMap(`${text}\n"a=b" = other\nincomplete`)).toEqual({ ...switchFlow.nodes.route.config.cases, '"a=b"': 'other' })
    })

    it('derives fork branch and join edges and saves them as the fork config, never as routes', () => {
        const forkFlow = {
            schema_version: '2',
            id: 'forking',
            title: 'Forking',
            nodes: {
                start: { kind: 'start', label: 'Start', config: { kind: 'start' }, routes: { next: { to: 'fork', description: 'Enter the first workflow step.' } } },
                fork: { kind: 'parallel', label: 'Fork', config: { kind: 'parallel', branches: ['a', 'b'], join: 'join', tolerate_branch_execution_errors: true } },
                a: { kind: 'agent_task', label: 'A', config: { kind: 'agent_task', prompt: 'A.' }, routes: { done: { to: 'join', description: 'Done.' } } },
                b: { kind: 'agent_task', label: 'B', config: { kind: 'agent_task', prompt: 'B.' }, routes: { done: { to: 'join', description: 'Done.' } } },
                join: { kind: 'fan_in', label: 'Join', config: { kind: 'fan_in' }, routes: { next: { to: 'done', description: 'All branches settled.' } } },
                done: exitNode('completed', 'success'),
            },
        }
        const hydrated = hydrate(forkFlow)
        expect(hydrated.edges.filter((edge) => edge.source === 'fork').map((edge) => [edge.target, edge.data?.kind])).toEqual([
            ['a', 'branch'],
            ['b', 'branch'],
            ['join', 'join'],
        ])
        const saved = buildFlow('forking', hydrated.nodes, hydrated.edges, hydrated.flowMetadata, hydrated.flow)
        expect(saved.nodes).toEqual(forkFlow.nodes)

        const withoutB = hydrated.edges.filter((edge) => !(edge.source === 'fork' && edge.target === 'b'))
        expect(buildFlow('forking', hydrated.nodes, withoutB, hydrated.flowMetadata, hydrated.flow).nodes.fork.config)
            .toEqual({ ...forkFlow.nodes.fork.config, branches: ['a'] })
    })
})
