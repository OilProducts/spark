import type { Edge, Node } from '@xyflow/react'

import { EDGE_RENDER_ROUTE_KEY } from './flowLayoutConstants.js'
import {
    routeFixedNodeGraph,
    type EdgeRoute,
    type NodeRect,
    type RouteSide,
    type RoutedPort,
} from './edgeRouting'
import type { SavedFlowLayoutEdgeV1, SavedFlowLayoutV1 } from './flowLayoutPersistence'
import { getNodeStyleDimension } from './workflowNodeShape'

const DEFAULT_NODE_WIDTH = 220
const DEFAULT_NODE_HEIGHT = 110

export { EDGE_RENDER_ROUTE_KEY } from './flowLayoutConstants.js'

export type EdgeLayoutAssignment = {
    edgeId: string
    layoutKey: string
    source: string
    target: string
    sourceSide: RouteSide
    targetSide: RouteSide
    sourceSlot: number
    targetSlot: number
    sourceSlotCount: number
    targetSlotCount: number
}

export type EdgeLayoutAssignments = {
    edgeIdToLayoutKey: Map<string, string>
    layoutKeyToEdgeId: Map<string, string>
    assignments: Record<string, EdgeLayoutAssignment>
}

export type LaidOutFlowGraph = {
    nodes: Node[]
    edges: Edge[]
    layout: SavedFlowLayoutV1
    edgeIdToLayoutKey: Map<string, string>
}

export function readNodeRect(node: Node): NodeRect {
    return {
        x: node.position.x,
        y: node.position.y,
        width: node.width ?? getNodeStyleDimension(node.style?.width) ?? DEFAULT_NODE_WIDTH,
        height: node.height ?? getNodeStyleDimension(node.style?.height) ?? DEFAULT_NODE_HEIGHT,
    }
}

function getNodeCenter(rect: NodeRect): { x: number; y: number } {
    return {
        x: rect.x + rect.width / 2,
        y: rect.y + rect.height / 2,
    }
}

const OPPOSITE_SIDE: Record<RouteSide, RouteSide> = { top: 'bottom', right: 'left', bottom: 'top', left: 'right' }

// Flows lay out top-down, so an edge to a node wholly below its source leaves from the bottom
// and enters at the top; turning sideways would cut through that node's neighbours.
function chooseDefaultSourceSide(sourceRect: NodeRect, targetRect: NodeRect): RouteSide {
    if (targetRect.y >= sourceRect.y + sourceRect.height) {
        return 'bottom'
    }
    const sourceCenter = getNodeCenter(sourceRect)
    const targetCenter = getNodeCenter(targetRect)
    const dx = targetCenter.x - sourceCenter.x
    const dy = targetCenter.y - sourceCenter.y

    if (Math.abs(dx) >= Math.abs(dy)) {
        return dx >= 0 ? 'right' : 'left'
    }
    return dy >= 0 ? 'bottom' : 'top'
}

function chooseDefaultTargetSide(sourceRect: NodeRect, targetRect: NodeRect): RouteSide {
    return OPPOSITE_SIDE[chooseDefaultSourceSide(sourceRect, targetRect)]
}

function getProjectionForSide(_referenceRect: NodeRect, otherRect: NodeRect, side: RouteSide): number {
    const otherCenter = getNodeCenter(otherRect)
    if (side === 'top' || side === 'bottom') {
        return otherCenter.x
    }
    return otherCenter.y
}

// Where a port goes in its side's order, from how its route leaves the side: routes that turn
// toward the side's start come first, the one that turns soonest outermost; then routes that run
// straight on; then routes that turn toward the side's end, the one that turns soonest outermost.
// In that order no route's turn crosses another's straight run. Without a route, the side of the
// node's centre its other end lies on stands in for the turn.
const TURN_RANK = 1e6
function getPortOrderKey(
    route: EdgeRoute | null | undefined,
    fromEnd: 'source' | 'target',
    side: RouteSide,
    nodeRect: NodeRect | undefined,
    projection: number,
): number {
    const [along, across] = side === 'top' || side === 'bottom' ? ['x', 'y'] as const : ['y', 'x'] as const
    const points = route && route.length >= 2 ? (fromEnd === 'source' ? route : [...route].reverse()) : null
    if (!points) {
        const center = nodeRect ? getNodeCenter(nodeRect)[along] : projection
        return Math.sign(projection - center) * (TURN_RANK - 1e5)
    }
    const port = points[0]
    // A jog of a few pixels, such as a shared line's spread, is not a turn.
    const turn = points.find((point) => Math.abs(point[along] - port[along]) > 16)
    if (!turn) {
        return 0
    }
    const depth = Math.abs(turn[across] - port[across])
    return turn[along] < port[along] ? -TURN_RANK + depth : TURN_RANK - depth
}

export function computeFlowTopologyStamp(nodes: Node[], edges: Edge[]): string {
    const nodeIds = nodes.map((node) => node.id).sort()
    const edgePairs = edges
        .map((edge) => `${edge.source}->${edge.target}`)
        .sort()
    return JSON.stringify({
        nodes: nodeIds,
        edges: edgePairs,
    })
}

export function buildEdgeLayoutKeyMap(edges: Edge[]): Map<string, string> {
    const countsByPair = new Map<string, number>()
    return new Map(
        edges.map((edge) => {
            const pairKey = `${edge.source}->${edge.target}`
            const occurrence = countsByPair.get(pairKey) ?? 0
            countsByPair.set(pairKey, occurrence + 1)
            return [edge.id, `${pairKey}#${occurrence}`] as const
        }),
    )
}

function applySavedNodePositions(nodes: Node[], savedLayout?: SavedFlowLayoutV1 | null, forceFreshLayout = false): Node[] {
    if (!savedLayout || forceFreshLayout) {
        return nodes
    }

    return nodes.map((node) => {
        const savedPosition = savedLayout.nodePositions[node.id]
        if (!savedPosition) {
            return node
        }
        return {
            ...node,
            position: {
                x: savedPosition.x,
                y: savedPosition.y,
            },
        }
    })
}

function compareLayoutGroupEntry(
    left: { orderKey: number; projection: number; edgeId: string },
    right: { orderKey: number; projection: number; edgeId: string },
): number {
    if (left.orderKey !== right.orderKey) {
        return left.orderKey - right.orderKey
    }
    if (left.projection !== right.projection) {
        return left.projection - right.projection
    }
    return left.edgeId.localeCompare(right.edgeId)
}

export function buildEdgeLayoutAssignments(
    nodes: Node[],
    edges: Edge[],
    savedLayout?: SavedFlowLayoutV1 | null,
    sideIntents?: Record<string, {
        sourceSide?: RouteSide
        targetSide?: RouteSide
    }>,
    headingRoutes?: Record<string, EdgeRoute>,
): EdgeLayoutAssignments {
    const edgeIdToLayoutKey = buildEdgeLayoutKeyMap(edges)
    const layoutKeyToEdgeId = new Map(
        [...edgeIdToLayoutKey.entries()].map(([edgeId, layoutKey]) => [layoutKey, edgeId] as const),
    )
    const nodeRects = new Map(nodes.map((node) => [node.id, readNodeRect(node)]))

    const baseAssignments = edges.map((edge) => {
        const layoutKey = edgeIdToLayoutKey.get(edge.id) ?? `${edge.source}->${edge.target}#0`
        const sourceRect = nodeRects.get(edge.source)
        const targetRect = nodeRects.get(edge.target)
        const savedEdgeLayout = savedLayout?.edgeLayouts[layoutKey]
        const sourceIntent = sideIntents?.[layoutKey]?.sourceSide
        const targetIntent = sideIntents?.[layoutKey]?.targetSide
        const sourceSide = sourceIntent ?? savedEdgeLayout?.sourceSide ?? (
            sourceRect && targetRect
                ? chooseDefaultSourceSide(sourceRect, targetRect)
                : 'bottom'
        )
        // A self-loop leaves and re-enters on its source side.
        const targetSide = targetIntent ?? savedEdgeLayout?.targetSide ?? (
            edge.source === edge.target
                ? sourceSide
                : sourceRect && targetRect
                    ? chooseDefaultTargetSide(sourceRect, targetRect)
                    : 'top'
        )

        const sourceProjection = sourceRect && targetRect ? getProjectionForSide(sourceRect, targetRect, sourceSide) : 0
        const targetProjection = sourceRect && targetRect ? getProjectionForSide(targetRect, sourceRect, targetSide) : 0
        // Ports on a side follow how their routes leave it, from this layout's first routing or the
        // last one, while its sides hold; a self-loop keeps its two ends in order on its one side.
        const savedRoute = savedEdgeLayout?.sourceSide === sourceSide && savedEdgeLayout.targetSide === targetSide
            ? savedEdgeLayout.route
            : undefined
        const headingRoute = edge.source === edge.target ? null : headingRoutes?.[layoutKey] ?? savedRoute

        return {
            edgeId: edge.id,
            layoutKey,
            source: edge.source,
            target: edge.target,
            sourceSide,
            targetSide,
            sourceProjection,
            targetProjection,
            sourceOrderKey: getPortOrderKey(headingRoute, 'source', sourceSide, sourceRect, sourceProjection),
            targetOrderKey: getPortOrderKey(headingRoute, 'target', targetSide, targetRect, targetProjection),
        }
    })

    // Edges leaving and entering one side of a node share its slots, so an outgoing and an
    // incoming edge never meet at the same point.
    const sideGroups = new Map<string, Array<{ slotKey: string; orderKey: number; projection: number; edgeId: string }>>()
    const addToSideGroup = (groupKey: string, slotKey: string, orderKey: number, projection: number, edgeId: string) => {
        const entries = sideGroups.get(groupKey) ?? []
        entries.push({ slotKey, orderKey, projection, edgeId })
        sideGroups.set(groupKey, entries)
    }

    baseAssignments.forEach((assignment) => {
        addToSideGroup(`${assignment.source}:${assignment.sourceSide}`, `${assignment.layoutKey}:source`, assignment.sourceOrderKey, assignment.sourceProjection, assignment.edgeId)
        addToSideGroup(`${assignment.target}:${assignment.targetSide}`, `${assignment.layoutKey}:target`, assignment.targetOrderKey, assignment.targetProjection, assignment.edgeId)
    })

    const slotByKey = new Map<string, { slot: number; slotCount: number }>()
    sideGroups.forEach((entries) => {
        const sortedEntries = [...entries].sort(compareLayoutGroupEntry)
        sortedEntries.forEach((entry, index) => {
            slotByKey.set(entry.slotKey, {
                slot: index,
                slotCount: sortedEntries.length,
            })
        })
    })

    const assignments = Object.fromEntries(
        baseAssignments.map((assignment) => {
            const sourceSlot = slotByKey.get(`${assignment.layoutKey}:source`) ?? { slot: 0, slotCount: 1 }
            const targetSlot = slotByKey.get(`${assignment.layoutKey}:target`) ?? { slot: 0, slotCount: 1 }
            return [assignment.layoutKey, {
                edgeId: assignment.edgeId,
                layoutKey: assignment.layoutKey,
                source: assignment.source,
                target: assignment.target,
                sourceSide: assignment.sourceSide,
                targetSide: assignment.targetSide,
                sourceSlot: sourceSlot.slot,
                targetSlot: targetSlot.slot,
                sourceSlotCount: sourceSlot.slotCount,
                targetSlotCount: targetSlot.slotCount,
            } satisfies EdgeLayoutAssignment] as const
        }),
    )

    return {
        edgeIdToLayoutKey,
        layoutKeyToEdgeId,
        assignments,
    }
}

function buildRouterPort(side: RouteSide, slot: number, slotCount: number): RoutedPort {
    return {
        side,
        slot,
        slotCount,
    }
}

export function routeEdgeLayoutAssignments(
    nodes: Node[],
    assignments: Record<string, EdgeLayoutAssignment>,
    previousLayout?: SavedFlowLayoutV1 | null,
    layoutKeys?: Iterable<string>,
): Record<string, EdgeRoute> {
    return routeFixedNodeGraph(
        buildFixedNodeRouterRequest(nodes, assignments, previousLayout, layoutKeys),
    ).routes
}

export function buildFixedNodeRouterRequest(
    nodes: Node[],
    assignments: Record<string, EdgeLayoutAssignment>,
    previousLayout?: SavedFlowLayoutV1 | null,
    layoutKeys?: Iterable<string>,
) {
    const requestedLayoutKeys = layoutKeys ? new Set(layoutKeys) : null
    return {
        nodes: nodes.map((node) => ({
            id: node.id,
            rect: readNodeRect(node),
        })),
        edges: Object.values(assignments)
            .filter((assignment) => !requestedLayoutKeys || requestedLayoutKeys.has(assignment.layoutKey))
            .map((assignment) => ({
                id: assignment.layoutKey,
                source: assignment.source,
                target: assignment.target,
                sourcePort: buildRouterPort(
                    assignment.sourceSide,
                    assignment.sourceSlot,
                    assignment.sourceSlotCount,
                ),
                targetPort: buildRouterPort(
                    assignment.targetSide,
                    assignment.targetSlot,
                    assignment.targetSlotCount,
                ),
                previousRoute: previousLayout?.edgeLayouts[assignment.layoutKey]?.route ?? null,
            })),
    }
}

export function edgeLayoutAssignmentsDiffer(
    savedEdgeLayout: SavedFlowLayoutEdgeV1 | null | undefined,
    assignment: EdgeLayoutAssignment,
): boolean {
    if (!savedEdgeLayout) {
        return true
    }

    return (
        savedEdgeLayout.sourceSide !== assignment.sourceSide
        || savedEdgeLayout.targetSide !== assignment.targetSide
        || savedEdgeLayout.sourceSlot !== assignment.sourceSlot
        || savedEdgeLayout.targetSlot !== assignment.targetSlot
    )
}

export function buildSavedFlowLayout(
    nodes: Node[],
    assignments: Record<string, EdgeLayoutAssignment>,
    routedEdges: Record<string, EdgeRoute>,
    topologyStamp: string,
    previousLayout?: SavedFlowLayoutV1 | null,
): SavedFlowLayoutV1 {
    return {
        version: 1,
        topologyStamp,
        nodePositions: Object.fromEntries(
            nodes.map((node) => [node.id, {
                x: node.position.x,
                y: node.position.y,
            }] as const),
        ),
        edgeLayouts: Object.fromEntries(
            Object.entries(assignments).flatMap(([layoutKey, assignment]) => {
                const previousRoute = previousLayout?.edgeLayouts[layoutKey]?.route
                const route = routedEdges[layoutKey] ?? previousRoute
                if (!route) {
                    return []
                }
                return [[layoutKey, {
                    sourceSide: assignment.sourceSide,
                    targetSide: assignment.targetSide,
                    sourceSlot: assignment.sourceSlot,
                    targetSlot: assignment.targetSlot,
                    route,
                } satisfies SavedFlowLayoutEdgeV1] as const]
            }),
        ),
        viewport: previousLayout?.viewport,
    }
}

export function attachRenderRoutesToEdges(
    edges: Edge[],
    edgeIdToLayoutKey: Map<string, string>,
    layout: SavedFlowLayoutV1,
): Edge[] {
    return edges.map((edge) => {
        const layoutKey = edgeIdToLayoutKey.get(edge.id)
        const route = layoutKey ? layout.edgeLayouts[layoutKey]?.route : null
        const nextData: Record<string, unknown> = {
            ...(edge.data ?? {}),
        }
        if (route) {
            nextData[EDGE_RENDER_ROUTE_KEY] = route
        } else {
            delete nextData[EDGE_RENDER_ROUTE_KEY]
        }

        return {
            ...edge,
            data: Object.keys(nextData).length > 0 ? nextData : undefined,
        }
    })
}

function canReuseSavedRoutes(
    assignments: Record<string, EdgeLayoutAssignment>,
    savedLayout?: SavedFlowLayoutV1 | null,
): boolean {
    if (!savedLayout) {
        return false
    }

    return Object.values(assignments).every((assignment) => {
        const savedEdgeLayout = savedLayout.edgeLayouts[assignment.layoutKey]
        return (
            savedEdgeLayout
            && !edgeLayoutAssignmentsDiffer(savedEdgeLayout, assignment)
            && savedEdgeLayout.route.length >= 2
        )
    })
}

export function buildFlowLayoutFromNodesAndEdges(
    nodes: Node[],
    edges: Edge[],
    options?: {
        savedLayout?: SavedFlowLayoutV1 | null
        forceFreshLayout?: boolean
    },
): LaidOutFlowGraph {
    const topologyStamp = computeFlowTopologyStamp(nodes, edges)
    const positionedNodes = applySavedNodePositions(nodes, options?.savedLayout, options?.forceFreshLayout ?? false)
    const savedLayout = options?.forceFreshLayout ? null : options?.savedLayout
    let { assignments, edgeIdToLayoutKey } = buildEdgeLayoutAssignments(positionedNodes, edges, savedLayout)

    let routes: Record<string, EdgeRoute>
    if (
        savedLayout
        && savedLayout.topologyStamp === topologyStamp
        && canReuseSavedRoutes(assignments, savedLayout)
    ) {
        routes = Object.fromEntries(
            Object.keys(assignments).flatMap((layoutKey) => {
                const savedEdgeLayout = savedLayout.edgeLayouts[layoutKey]
                return savedEdgeLayout ? [[layoutKey, savedEdgeLayout.route] as const] : []
            }),
        )
    } else {
        // Ports first take their side's slots in the order of their targets; route, then order each
        // side's slots by where the routes really head and route again, so exits do not cross.
        const firstRoutes = routeEdgeLayoutAssignments(positionedNodes, assignments, options?.savedLayout)
        ;({ assignments, edgeIdToLayoutKey } = buildEdgeLayoutAssignments(positionedNodes, edges, savedLayout, undefined, firstRoutes))
        routes = routeEdgeLayoutAssignments(positionedNodes, assignments, options?.savedLayout)
    }

    const layout = buildSavedFlowLayout(
        positionedNodes,
        assignments,
        routes,
        topologyStamp,
        options?.forceFreshLayout ? null : options?.savedLayout,
    )

    return {
        nodes: positionedNodes,
        edges: attachRenderRoutesToEdges(edges, edgeIdToLayoutKey, layout),
        layout,
        edgeIdToLayoutKey,
    }
}
