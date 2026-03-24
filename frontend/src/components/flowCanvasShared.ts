import type { Edge, Node } from '@xyflow/react'
import ELK from 'elkjs/lib/elk.bundled.js'

import type { GraphAttrs, UiDefaults } from '@/store'
import type { PreviewResponsePayload } from '@/lib/attractorClient'
import { buildCanonicalFlowModelFromPreviewGraph } from '@/lib/canonicalFlowModel'
import {
    getNodeStyleDimension,
    getReactFlowNodeTypeForShape,
    getShapeNodeStyle,
} from '@/lib/workflowNodeShape'

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
    'elk.layered.spacing.nodeNodeBetweenLayers': '30',
    'elk.spacing.nodeNode': '20',
    'elk.layered.cycleBreaking.strategy': 'DEPTH_FIRST',
}

const elk = new ELK()

export type PreviewResponse = PreviewResponsePayload

export type HydratedFlowGraph = {
    nodes: Node[]
    edges: Edge[]
    graphAttrs: GraphAttrs
    defaults: ReturnType<typeof buildCanonicalFlowModelFromPreviewGraph>['defaults']
    subgraphs: ReturnType<typeof buildCanonicalFlowModelFromPreviewGraph>['subgraphs']
}

export function normalizeLegacyDot(content: string): string {
    return content.replace(/\blabel=label=/g, 'label=')
}

export async function layoutWithElk(nodes: Node[], edges: Edge[]): Promise<Node[]> {
    const graph = {
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

    const layout = await elk.layout(graph)
    const layoutMap = new Map((layout.children ?? []).map((child) => [child.id, child]))

    return nodes.map((node) => {
        const layoutNode = layoutMap.get(node.id)
        if (!layoutNode) return node
        return {
            ...node,
            position: {
                x: layoutNode.x ?? node.position.x,
                y: layoutNode.y ?? node.position.y,
            },
        }
    })
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
    uiDefaults: UiDefaults,
    sourceDot?: string,
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
    const shouldSeed = (value?: string | null) =>
        value === undefined || value === null || value === ''

    if (preview.graph.graph_attrs) {
        if (shouldSeed(nextGraphAttrs.ui_default_llm_model) && uiDefaults.llm_model) {
            nextGraphAttrs.ui_default_llm_model = uiDefaults.llm_model
        }
        if (shouldSeed(nextGraphAttrs.ui_default_llm_provider) && uiDefaults.llm_provider) {
            nextGraphAttrs.ui_default_llm_provider = uiDefaults.llm_provider
        }
        if (shouldSeed(nextGraphAttrs.ui_default_reasoning_effort) && uiDefaults.reasoning_effort) {
            nextGraphAttrs.ui_default_reasoning_effort = uiDefaults.reasoning_effort
        }
    }

    const nodes: Node[] = canonicalModel.nodes.map((n, i: number) => {
        const shape = typeof n.attrs.shape === 'string' ? n.attrs.shape : 'box'
        return {
            id: n.id,
            type: getReactFlowNodeTypeForShape(shape),
            position: { x: 250, y: i * 150 },
            style: getShapeNodeStyle(shape),
            data: {
                ...n.attrs,
                label: typeof n.attrs.label === 'string' ? n.attrs.label : n.id,
                shape,
                prompt: typeof n.attrs.prompt === 'string' ? n.attrs.prompt : '',
                'tool.command': typeof n.attrs['tool.command'] === 'string' ? n.attrs['tool.command'] : '',
                'tool.hooks.pre': typeof n.attrs['tool.hooks.pre'] === 'string' ? n.attrs['tool.hooks.pre'] : '',
                'tool.hooks.post': typeof n.attrs['tool.hooks.post'] === 'string' ? n.attrs['tool.hooks.post'] : '',
                'tool.artifacts.paths': typeof n.attrs['tool.artifacts.paths'] === 'string'
                    ? n.attrs['tool.artifacts.paths']
                    : '',
                'tool.artifacts.stdout': typeof n.attrs['tool.artifacts.stdout'] === 'string'
                    ? n.attrs['tool.artifacts.stdout']
                    : '',
                'tool.artifacts.stderr': typeof n.attrs['tool.artifacts.stderr'] === 'string'
                    ? n.attrs['tool.artifacts.stderr']
                    : '',
                join_policy: typeof n.attrs.join_policy === 'string' ? n.attrs.join_policy : 'wait_all',
                error_policy: typeof n.attrs.error_policy === 'string' ? n.attrs.error_policy : 'continue',
                max_parallel: typeof n.attrs.max_parallel === 'number' || typeof n.attrs.max_parallel === 'string'
                    ? n.attrs.max_parallel
                    : 4,
                type: typeof n.attrs.type === 'string' ? n.attrs.type : '',
                max_retries: typeof n.attrs.max_retries === 'number' || typeof n.attrs.max_retries === 'string'
                    ? n.attrs.max_retries
                    : '',
                goal_gate: n.attrs.goal_gate === true || n.attrs.goal_gate === 'true',
                retry_target: typeof n.attrs.retry_target === 'string' ? n.attrs.retry_target : '',
                fallback_retry_target: typeof n.attrs.fallback_retry_target === 'string' ? n.attrs.fallback_retry_target : '',
                fidelity: typeof n.attrs.fidelity === 'string' ? n.attrs.fidelity : '',
                thread_id: typeof n.attrs.thread_id === 'string' ? n.attrs.thread_id : '',
                class: typeof n.attrs.class === 'string' ? n.attrs.class : '',
                timeout: typeof n.attrs.timeout === 'string' ? n.attrs.timeout : '',
                llm_model: typeof n.attrs.llm_model === 'string' ? n.attrs.llm_model : '',
                llm_provider: typeof n.attrs.llm_provider === 'string' ? n.attrs.llm_provider : '',
                reasoning_effort: typeof n.attrs.reasoning_effort === 'string' ? n.attrs.reasoning_effort : '',
                auto_status: n.attrs.auto_status === true || n.attrs.auto_status === 'true',
                allow_partial: n.attrs.allow_partial === true || n.attrs.allow_partial === 'true',
                'manager.poll_interval': typeof n.attrs['manager.poll_interval'] === 'string'
                    ? n.attrs['manager.poll_interval']
                    : '',
                'manager.max_cycles': typeof n.attrs['manager.max_cycles'] === 'number'
                    || typeof n.attrs['manager.max_cycles'] === 'string'
                    ? n.attrs['manager.max_cycles']
                    : '',
                'manager.stop_condition': typeof n.attrs['manager.stop_condition'] === 'string'
                    ? n.attrs['manager.stop_condition']
                    : '',
                'manager.actions': typeof n.attrs['manager.actions'] === 'string' ? n.attrs['manager.actions'] : '',
                'human.default_choice': typeof n.attrs['human.default_choice'] === 'string'
                    ? n.attrs['human.default_choice']
                    : '',
                status: 'idle',
            },
        }
    })

    const edges: Edge[] = canonicalModel.edges.map((e, i: number) => ({
        id: `e-${e.source}-${e.target}-${i}`,
        source: e.source,
        target: e.target,
        type: EDGE_TYPE,
        className: EDGE_CLASS,
        interactionWidth: EDGE_INTERACTION_WIDTH,
        label: typeof e.attrs.label === 'string' ? e.attrs.label : undefined,
        data: {
            ...e.attrs,
            label: typeof e.attrs.label === 'string' ? e.attrs.label : '',
            condition: typeof e.attrs.condition === 'string' ? e.attrs.condition : '',
            weight: typeof e.attrs.weight === 'number' || typeof e.attrs.weight === 'string' ? e.attrs.weight : '',
            fidelity: typeof e.attrs.fidelity === 'string' ? e.attrs.fidelity : '',
            thread_id: typeof e.attrs.thread_id === 'string' ? e.attrs.thread_id : '',
            loop_restart: e.attrs.loop_restart === true || e.attrs.loop_restart === 'true',
        },
    }))

    return {
        nodes,
        edges,
        graphAttrs: nextGraphAttrs,
        defaults: canonicalModel.defaults,
        subgraphs: canonicalModel.subgraphs,
    }
}
