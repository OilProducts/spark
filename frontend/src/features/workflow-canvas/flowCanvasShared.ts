import type { Edge, Node } from '@xyflow/react'
import ELK from 'elkjs/lib/elk.bundled.js'

import type { GraphAttrs, UiDefaults } from '@/store'
import {
    buildCanonicalFlowModelFromPreviewGraph,
    type CanonicalFlowEdge,
    type CanonicalFlowNode,
    type CanonicalPreviewGraphPayload,
} from '@/lib/canonicalFlowModel'
import {
    getNodeStyleDimension,
    getReactFlowNodeTypeForShape,
    getShapeNodeStyle,
} from '@/lib/workflowNodeShape'
import { buildFlowLayoutFromNodesAndEdges } from '@/lib/flowLayout'
import type { SavedFlowLayoutV1 } from '@/lib/flowLayoutPersistence'
import {
    attachDerivedPreviewMeta,
    type DerivedPreviewMeta,
} from '@/features/workflow-canvas/derivedPreview'

import {
    ConditionalNode,
    ExitNode,
    FanInNode,
    HumanGateNode,
    ManagerNode,
    ParallelNode,
    StartNode,
    TaskNode,
    ToolNode,
} from './TaskNode'
import { ValidationEdge } from './ValidationEdge'
import type { WorkflowCanvasPreviewResponse } from './model/types'

export const nodeTypes = {
    customTask: TaskNode,
    startNode: StartNode,
    exitNode: ExitNode,
    taskNode: TaskNode,
    humanGateNode: HumanGateNode,
    conditionalNode: ConditionalNode,
    parallelNode: ParallelNode,
    fanInNode: FanInNode,
    toolNode: ToolNode,
    managerNode: ManagerNode,
}

export const edgeTypes = {
    validation: ValidationEdge,
}

export const EDGE_TYPE: Edge['type'] = 'validation'
export const EDGE_CLASS = 'flow-edge'
export const EDGE_INTERACTION_WIDTH = 16

const DEFAULT_NODE_WIDTH = 220
const DEFAULT_NODE_HEIGHT = 110
const ELK_OPTIONS = {
    'elk.algorithm': 'layered',
    'elk.direction': 'DOWN',
    'elk.edgeRouting': 'ORTHOGONAL',
    'elk.layered.spacing.nodeNodeBetweenLayers': '30',
    'elk.spacing.nodeNode': '20',
    'elk.layered.cycleBreaking.strategy': 'DEPTH_FIRST',
}

const elk = new ELK()

export type PreviewResponse = WorkflowCanvasPreviewResponse

export type HydratedFlowGraph = {
    nodes: Node[]
    edges: Edge[]
    graphAttrs: GraphAttrs
    defaults: ReturnType<typeof buildCanonicalFlowModelFromPreviewGraph>['defaults']
    subgraphs: ReturnType<typeof buildCanonicalFlowModelFromPreviewGraph>['subgraphs']
}

export type LaidOutFlowGraph = {
    nodes: Node[]
    edges: Edge[]
    layout: SavedFlowLayoutV1
    edgeIdToLayoutKey: Map<string, string>
}

export type BuildHydratedFlowGraphOptions = {
    expandChildren?: boolean
}

type ParsedChildPreview = {
    flowName: string
    flowPath: string
    flowLabel: string
    graph: CanonicalPreviewGraphPayload
}

const DERIVED_CHILD_EDGE_STYLE: Edge['style'] = {
    opacity: 0.62,
    stroke: 'hsl(var(--muted-foreground) / 0.45)',
}

const DERIVED_CHILD_LINK_EDGE_STYLE: Edge['style'] = {
    opacity: 0.8,
    stroke: 'hsl(var(--muted-foreground) / 0.6)',
    strokeDasharray: '6 6',
}

function asRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object') {
        return null
    }
    return value as Record<string, unknown>
}

export function normalizeLegacyDot(content: string): string {
    return content.replace(/\blabel=label=/g, 'label=')
}

function buildHydratedNode(
    node: CanonicalFlowNode,
    index: number,
    derivedPreviewMeta?: DerivedPreviewMeta,
): Node {
    const shape = typeof node.attrs.shape === 'string' ? node.attrs.shape : 'box'
    const nodeData: Record<string, unknown> = {
        ...node.attrs,
        label: typeof node.attrs.label === 'string' ? node.attrs.label : node.id,
        shape,
        prompt: typeof node.attrs.prompt === 'string' ? node.attrs.prompt : '',
        'tool.command': typeof node.attrs['tool.command'] === 'string' ? node.attrs['tool.command'] : '',
        'tool.hooks.pre': typeof node.attrs['tool.hooks.pre'] === 'string' ? node.attrs['tool.hooks.pre'] : '',
        'tool.hooks.post': typeof node.attrs['tool.hooks.post'] === 'string' ? node.attrs['tool.hooks.post'] : '',
        'tool.artifacts.paths': typeof node.attrs['tool.artifacts.paths'] === 'string'
            ? node.attrs['tool.artifacts.paths']
            : '',
        'tool.artifacts.stdout': typeof node.attrs['tool.artifacts.stdout'] === 'string'
            ? node.attrs['tool.artifacts.stdout']
            : '',
        'tool.artifacts.stderr': typeof node.attrs['tool.artifacts.stderr'] === 'string'
            ? node.attrs['tool.artifacts.stderr']
            : '',
        type: typeof node.attrs.type === 'string' ? node.attrs.type : '',
        max_retries: typeof node.attrs.max_retries === 'number' || typeof node.attrs.max_retries === 'string'
            ? node.attrs.max_retries
            : '',
        retry_target: typeof node.attrs.retry_target === 'string' ? node.attrs.retry_target : '',
        fallback_retry_target: typeof node.attrs.fallback_retry_target === 'string' ? node.attrs.fallback_retry_target : '',
        fidelity: typeof node.attrs.fidelity === 'string' ? node.attrs.fidelity : '',
        thread_id: typeof node.attrs.thread_id === 'string' ? node.attrs.thread_id : '',
        class: typeof node.attrs.class === 'string' ? node.attrs.class : '',
        timeout: typeof node.attrs.timeout === 'string' ? node.attrs.timeout : '',
        llm_model: typeof node.attrs.llm_model === 'string' ? node.attrs.llm_model : '',
        llm_provider: typeof node.attrs.llm_provider === 'string' ? node.attrs.llm_provider : '',
        llm_profile: typeof node.attrs.llm_profile === 'string' ? node.attrs.llm_profile : '',
        reasoning_effort: typeof node.attrs.reasoning_effort === 'string' ? node.attrs.reasoning_effort : '',
        'manager.poll_interval': typeof node.attrs['manager.poll_interval'] === 'string'
            ? node.attrs['manager.poll_interval']
            : '',
        'manager.max_cycles': typeof node.attrs['manager.max_cycles'] === 'number'
            || typeof node.attrs['manager.max_cycles'] === 'string'
            ? node.attrs['manager.max_cycles']
            : '',
        'manager.stop_condition': typeof node.attrs['manager.stop_condition'] === 'string'
            ? node.attrs['manager.stop_condition']
            : '',
        'manager.actions': typeof node.attrs['manager.actions'] === 'string' ? node.attrs['manager.actions'] : '',
        'human.default_choice': typeof node.attrs['human.default_choice'] === 'string'
            ? node.attrs['human.default_choice']
            : '',
        status: 'idle',
    }

    const nextNode: Node = {
        id: node.id,
        type: getReactFlowNodeTypeForShape(shape),
        position: { x: 250, y: index * 150 },
        style: getShapeNodeStyle(shape),
        data: derivedPreviewMeta ? attachDerivedPreviewMeta(nodeData, derivedPreviewMeta) : nodeData,
    }
    if (derivedPreviewMeta) {
        nextNode.draggable = false
        nextNode.selectable = false
        nextNode.connectable = false
        nextNode.focusable = false
        nextNode.deletable = false
    }
    return nextNode
}

function buildHydratedEdge(
    edge: CanonicalFlowEdge,
    index: number,
    options?: {
        id?: string
        source?: string
        target?: string
        derivedPreviewMeta?: DerivedPreviewMeta
        style?: Edge['style']
        hidden?: boolean
    },
): Edge {
    const edgeData: Record<string, unknown> = {
        ...edge.attrs,
        label: typeof edge.attrs.label === 'string' ? edge.attrs.label : '',
        condition: typeof edge.attrs.condition === 'string' ? edge.attrs.condition : '',
        weight: typeof edge.attrs.weight === 'number' || typeof edge.attrs.weight === 'string' ? edge.attrs.weight : '',
        fidelity: typeof edge.attrs.fidelity === 'string' ? edge.attrs.fidelity : '',
        thread_id: typeof edge.attrs.thread_id === 'string' ? edge.attrs.thread_id : '',
        loop_restart: edge.attrs.loop_restart === true || edge.attrs.loop_restart === 'true',
    }
    const nextEdge: Edge = {
        id: options?.id ?? `e-${edge.source}-${edge.target}-${index}`,
        source: options?.source ?? edge.source,
        target: options?.target ?? edge.target,
        type: EDGE_TYPE,
        className: EDGE_CLASS,
        interactionWidth: EDGE_INTERACTION_WIDTH,
        label: typeof edge.attrs.label === 'string' ? edge.attrs.label : undefined,
        data: options?.derivedPreviewMeta ? attachDerivedPreviewMeta(edgeData, options.derivedPreviewMeta) : edgeData,
        style: options?.style,
        hidden: options?.hidden,
    }
    if (options?.derivedPreviewMeta) {
        nextEdge.selectable = false
        nextEdge.focusable = false
        nextEdge.deletable = false
    }
    return nextEdge
}

function parseChildPreviewMap(value: unknown): Record<string, ParsedChildPreview> {
    const record = asRecord(value)
    if (!record) {
        return {}
    }

    return Object.fromEntries(
        Object.entries(record).flatMap(([managerNodeId, entryValue]) => {
            const entryRecord = asRecord(entryValue)
            const graphRecord = asRecord(entryRecord?.graph)
            if (!graphRecord) {
                return []
            }
            const nodes = Array.isArray(graphRecord.nodes) ? graphRecord.nodes : null
            const edges = Array.isArray(graphRecord.edges) ? graphRecord.edges : null
            if (!nodes || !edges) {
                return []
            }

            const childGraph: CanonicalPreviewGraphPayload = {
                nodes: nodes
                    .map((node) => asRecord(node))
                    .filter((node): node is Record<string, unknown> => node !== null),
                edges: edges
                    .map((edge) => asRecord(edge))
                    .filter((edge): edge is Record<string, unknown> => edge !== null),
                graph_attrs: asRecord(graphRecord.graph_attrs),
                defaults: asRecord(graphRecord.defaults),
                subgraphs: Array.isArray(graphRecord.subgraphs) ? graphRecord.subgraphs : undefined,
            }

            return [[managerNodeId, {
                flowName: typeof entryRecord?.flow_name === 'string' ? entryRecord.flow_name : 'child.dot',
                flowPath: typeof entryRecord?.flow_path === 'string' ? entryRecord.flow_path : '',
                flowLabel: typeof entryRecord?.flow_label === 'string'
                    ? entryRecord.flow_label
                    : typeof entryRecord?.flow_name === 'string'
                        ? entryRecord.flow_name
                        : 'Child Flow',
                graph: childGraph,
            } satisfies ParsedChildPreview]]
        }),
    )
}

function getChildClusterNodeId(managerNodeId: string): string {
    return `__child_preview_cluster__${managerNodeId}`
}

function getNamespacedChildNodeId(managerNodeId: string, childNodeId: string): string {
    return `__child_preview__${managerNodeId}__${childNodeId}`
}

function selectChildEntryNodeId(
    childNodes: CanonicalFlowNode[],
    childInDegree: Map<string, number>,
): string | null {
    const rootNodes = childNodes.filter((node) => (childInDegree.get(node.id) ?? 0) === 0)
    if (rootNodes.length === 0) {
        return childNodes[0]?.id ?? null
    }

    const explicitStartNode = rootNodes.find((node) => node.attrs.shape === 'Mdiamond')
    if (explicitStartNode) {
        return explicitStartNode.id
    }

    return rootNodes[0]?.id ?? null
}

function buildExpandedChildPreviewElements(
    flowName: string,
    parentNodes: Node[],
    childPreviews: Record<string, ParsedChildPreview>,
): { nodes: Node[]; edges: Edge[] } {
    const parentNodeIds = new Set(parentNodes.map((node) => node.id))
    const derivedNodes: Node[] = []
    const derivedEdges: Edge[] = []

    Object.entries(childPreviews).forEach(([managerNodeId, childPreview]) => {
        if (!parentNodeIds.has(managerNodeId)) {
            return
        }

        const childModel = buildCanonicalFlowModelFromPreviewGraph(
            `${flowName}:${managerNodeId}:${childPreview.flowName}`,
            childPreview.graph,
        )
        if (childModel.nodes.length === 0) {
            return
        }

        const clusterNodeId = getChildClusterNodeId(managerNodeId)
        const clusterNodeData = attachDerivedPreviewMeta(
            {
                label: `Child Flow Preview: ${childPreview.flowLabel}`,
                shape: 'box',
                prompt: childPreview.flowPath ? `Resolved from ${childPreview.flowPath}` : '',
                status: 'idle',
            },
            {
                kind: 'child-cluster',
                managerNodeId,
                readOnly: true,
            },
        )
        derivedNodes.push({
            id: clusterNodeId,
            type: getReactFlowNodeTypeForShape('box'),
            position: { x: 250, y: 0 },
            style: { ...getShapeNodeStyle('box'), width: 264, height: 76 },
            data: clusterNodeData,
            draggable: false,
            selectable: false,
            connectable: false,
            focusable: false,
            deletable: false,
        })

        const namespacedNodeIds = new Map<string, string>()
        childModel.nodes.forEach((childNode, index) => {
            const namespacedId = getNamespacedChildNodeId(managerNodeId, childNode.id)
            namespacedNodeIds.set(childNode.id, namespacedId)
            derivedNodes.push(buildHydratedNode(
                {
                    ...childNode,
                    id: namespacedId,
                },
                parentNodes.length + derivedNodes.length + index,
                {
                    kind: 'child-node',
                    managerNodeId,
                    originalNodeId: childNode.id,
                    readOnly: true,
                },
            ))
        })

        const childInDegree = new Map(childModel.nodes.map((node) => [node.id, 0]))
        childModel.edges.forEach((childEdge) => {
            childInDegree.set(childEdge.target, (childInDegree.get(childEdge.target) ?? 0) + 1)
            derivedEdges.push(buildHydratedEdge(
                childEdge,
                derivedEdges.length,
                {
                    id: `e-${managerNodeId}-${childEdge.source}-${childEdge.target}-${derivedEdges.length}`,
                    source: namespacedNodeIds.get(childEdge.source) ?? childEdge.source,
                    target: namespacedNodeIds.get(childEdge.target) ?? childEdge.target,
                    derivedPreviewMeta: {
                        kind: 'child-edge',
                        managerNodeId,
                        readOnly: true,
                    },
                    style: DERIVED_CHILD_EDGE_STYLE,
                },
            ))
        })

        const entryNodeId = selectChildEntryNodeId(childModel.nodes, childInDegree)
        const entryTargetId = entryNodeId ? namespacedNodeIds.get(entryNodeId) : null

        derivedEdges.push({
            id: `e-${managerNodeId}-child-preview-link`,
            source: managerNodeId,
            target: entryTargetId ?? clusterNodeId,
            type: EDGE_TYPE,
            className: EDGE_CLASS,
            interactionWidth: EDGE_INTERACTION_WIDTH,
            data: attachDerivedPreviewMeta({}, {
                kind: 'child-link',
                managerNodeId,
                readOnly: true,
            }),
            style: DERIVED_CHILD_LINK_EDGE_STYLE,
            selectable: false,
            focusable: false,
            deletable: false,
        })

        const rootNodeIds = childModel.nodes
            .filter((node) => (childInDegree.get(node.id) ?? 0) === 0)
            .map((node) => node.id)
        const anchorTargets = rootNodeIds.length > 0 ? rootNodeIds : [childModel.nodes[0].id]
        anchorTargets.forEach((rootNodeId, index) => {
            const targetId = namespacedNodeIds.get(rootNodeId)
            if (!targetId) {
                return
            }
            derivedEdges.push({
                id: `e-${managerNodeId}-child-preview-anchor-${index}`,
                source: clusterNodeId,
                target: targetId,
                type: EDGE_TYPE,
                className: EDGE_CLASS,
                interactionWidth: 0,
                data: attachDerivedPreviewMeta({}, {
                    kind: 'layout-anchor',
                    managerNodeId,
                    readOnly: true,
                }),
                hidden: true,
                selectable: false,
                focusable: false,
                deletable: false,
            })
        })
    })

    return {
        nodes: derivedNodes,
        edges: derivedEdges,
    }
}

function buildElkLayoutGraph(nodes: Node[], edges: Edge[]) {
    return {
        id: 'root',
        layoutOptions: ELK_OPTIONS,
        children: nodes.map((node) => ({
            id: node.id,
            width: node.width ?? getNodeStyleDimension(node.style?.width) ?? DEFAULT_NODE_WIDTH,
            height: node.height ?? getNodeStyleDimension(node.style?.height) ?? DEFAULT_NODE_HEIGHT,
        })),
        edges: edges.map((edge) => ({
            id: edge.id,
            sources: [edge.source],
            targets: [edge.target],
        })),
    }
}

type ElkLayoutNodeLike = {
    x?: number
    y?: number
}

async function computeElkLayout(nodes: Node[], edges: Edge[]) {
    const layout = await elk.layout(buildElkLayoutGraph(nodes, edges))
    const layoutMap = new Map(
        ((layout.children ?? []) as Array<ElkLayoutNodeLike & { id?: string }>)
            .filter((child): child is ElkLayoutNodeLike & { id: string } => typeof child.id === 'string')
            .map((child) => [child.id, child]),
    )

    return {
        layoutMap,
    }
}

export async function layoutWithElk(
    nodes: Node[],
    edges: Edge[],
    options?: {
        savedLayout?: SavedFlowLayoutV1 | null
        forceFreshLayout?: boolean
    },
): Promise<LaidOutFlowGraph> {
    const { layoutMap } = await computeElkLayout(nodes, edges)
    const positionedNodes = nodes.map((node) => {
        const layoutNode = layoutMap.get(node.id)
        if (!layoutNode) {
            return node
        }
        return {
            ...node,
            position: {
                x: layoutNode.x ?? node.position.x,
                y: layoutNode.y ?? node.position.y,
            },
        }
    })

    return buildFlowLayoutFromNodesAndEdges(positionedNodes, edges, options)
}

export function nowMs(): number {
    if (typeof performance !== 'undefined' && typeof performance.now === 'function') {
        return performance.now()
    }
    return Date.now()
}

export function buildHydratedFlowGraph(
    flowName: string,
    preview: PreviewResponse,
    _uiDefaults: UiDefaults,
    sourceDot?: string,
    options?: BuildHydratedFlowGraphOptions,
): HydratedFlowGraph | null {
    if (!preview.graph) {
        return null
    }

    const canonicalModel = buildCanonicalFlowModelFromPreviewGraph(
        flowName,
        preview.graph,
        sourceDot !== undefined ? { rawDot: sourceDot } : undefined,
    )
    const nextGraphAttrs: GraphAttrs = preview.graph.graph_attrs ? { ...canonicalModel.graphAttrs } : {}

    const parentNodes = canonicalModel.nodes.map((node, index) => buildHydratedNode(node, index))
    const parentEdges = canonicalModel.edges.map((edge, index) => buildHydratedEdge(edge, index))
    const childPreviewElements = options?.expandChildren
        ? buildExpandedChildPreviewElements(
            flowName,
            parentNodes,
            parseChildPreviewMap(preview.graph.child_previews),
        )
        : { nodes: [], edges: [] }

    return {
        nodes: [...parentNodes, ...childPreviewElements.nodes],
        edges: [...parentEdges, ...childPreviewElements.edges],
        graphAttrs: nextGraphAttrs,
        defaults: canonicalModel.defaults,
        subgraphs: canonicalModel.subgraphs,
    }
}
