const EMPTY_DIAGNOSTICS: Record<string, import('@/store').DiagnosticEntry[]> = {}
import { useCallback, type CSSProperties, type MouseEvent as ReactMouseEvent } from 'react'
import { BaseEdge, EdgeLabelRenderer, useReactFlow, useStore as useFlowStore, type Edge, type EdgeProps, type Node } from '@xyflow/react'

import {
    buildRoundedPolylinePath,
    getRouteMidpoint,
    normalizeRoute,
    type EdgeRoute,
    type EdgeRoutePoint,
} from '@/lib/edgeRouting'
import { EDGE_RENDER_ROUTE_KEY } from '@/lib/flowLayout'
import { useStore as useAppStore } from '@/store'

import { useCanvasSessionMode, useCanvasRunId } from './canvasSessionContext'
import { getDerivedPreviewMeta } from './derivedPreview'

function readRenderRoute(value: unknown): EdgeRoute | null {
    if (!Array.isArray(value)) {
        return null
    }

    const route = value
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

    return route.length >= 2 ? route : null
}

// A label's box at text-xs, estimated so labels can be placed before they render; max-w-44 wraps.
const LABEL_HEIGHT = 20
const labelWidth = (text: string) => Math.min(176, text.length * 6.6 + 14)

type LabelAnchor = { x: number; y: number }
type LabelBox = { left: number; right: number; top: number; bottom: number }

const labelBox = (anchor: LabelAnchor, width: number): LabelBox => ({
    left: anchor.x - width / 2,
    right: anchor.x + width / 2,
    top: anchor.y - LABEL_HEIGHT / 2,
    bottom: anchor.y + LABEL_HEIGHT / 2,
})

const segmentEntersBox = (from: EdgeRoutePoint, to: EdgeRoutePoint, box: LabelBox) => (
    Math.max(from.x, to.x) > box.left + 1 && Math.min(from.x, to.x) < box.right - 1
    && Math.max(from.y, to.y) > box.top + 1 && Math.min(from.y, to.y) < box.bottom - 1
)

/**
 * Where a route's label sits: on one of its horizontal runs, in the lane between node rows, or
 * else beside one of its vertical runs, wherever the label's box meets the fewest other routes
 * and placed labels; a run's middle and longer runs win ties.
 */
export function routeLabelAnchor(
    route: EdgeRoute,
    otherRoutes: EdgeRoute[] = [],
    placedLabels: LabelBox[] = [],
    width = 48,
): LabelAnchor {
    const best: { anchor: LabelAnchor | null; cost: number } = { anchor: null, cost: Infinity }
    const consider = (anchor: LabelAnchor, onSegment: number, preference: number) => {
        const box = labelBox(anchor, width)
        const routeHits = otherRoutes.reduce((hits, other) => hits + (other.slice(1).some((to, at) => segmentEntersBox(other[at], to, box)) ? 1 : 0), 0)
        const ownHits = route.slice(1).filter((to, at) => at !== onSegment && segmentEntersBox(route[at], to, box)).length
        // Placed labels and nodes: a label over either hides it.
        const labelHits = placedLabels.filter((other) => other.left < box.right && other.right > box.left && other.top < box.bottom && other.bottom > box.top).length
        // A hit outweighs any preference, so a clear spot always wins.
        const cost = (routeHits + ownHits + 2 * labelHits) * 1e6 + preference
        if (cost < best.cost) Object.assign(best, { anchor, cost })
    }
    route.slice(1).forEach((to, at) => {
        const from = route[at]
        const length = Math.hypot(to.x - from.x, to.y - from.y)
        if (Math.abs(to.y - from.y) < 1) {
            const left = Math.min(from.x, to.x)
            const right = Math.max(from.x, to.x)
            const middle = (left + right) / 2
            // ponytail: samples every 8px; labels are few, so this stays cheap on the largest flows.
            const first = Math.min(middle, left + width / 2)
            const last = Math.max(middle, right - width / 2)
            for (let x = first; x <= last + 0.5; x += 8) {
                consider({ x, y: from.y }, at, Math.abs(x - middle) - length)
            }
            consider({ x: middle, y: from.y }, at, -length)
        } else if (length >= 16) {
            const y = (from.y + to.y) / 2
            // Beside a vertical run ranks after every run between rows.
            consider({ x: from.x - 6 - width / 2, y }, at, 1e5 - length)
            consider({ x: from.x + 6 + width / 2, y }, at, 1e5 - length)
        }
    })
    return best.anchor ?? getRouteMidpoint(route)
}

/** Every labelled route's label anchor, placed in edge order so each label keeps clear of the nodes and the labels before it. */
function placeRouteLabels(edges: Edge[], nodes: Node[]): Map<string, LabelAnchor> {
    const routes = edges.flatMap((edge) => {
        const route = edge.hidden ? null : readRenderRoute((edge.data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY])
        return route ? [{ id: edge.id, route, label: edgeRouteLabel(edge.data) }] : []
    })
    const placed: LabelBox[] = nodes.flatMap((node) => {
        const width = node.measured?.width ?? node.width
        const height = node.measured?.height ?? node.height
        return node.hidden || !width || !height
            ? []
            : [{ left: node.position.x, right: node.position.x + width, top: node.position.y, bottom: node.position.y + height }]
    })
    const anchors = new Map<string, LabelAnchor>()
    for (const { id, route, label } of routes) {
        if (!label) continue
        const width = labelWidth(label)
        const anchor = routeLabelAnchor(route, routes.filter((other) => other.id !== id).map((other) => other.route), placed, width)
        anchors.set(id, anchor)
        placed.push(labelBox(anchor, width))
    }
    return anchors
}

// One placement per graph, shared by every labelled edge that renders from it.
const placedLabelsByGraph = new WeakMap<Edge[], WeakMap<Node[], Map<string, LabelAnchor>>>()
function routeLabelAnchors(edges: Edge[], nodes: Node[]): Map<string, LabelAnchor> {
    const byNodes = placedLabelsByGraph.get(edges) ?? new WeakMap<Node[], Map<string, LabelAnchor>>()
    placedLabelsByGraph.set(edges, byNodes)
    let anchors = byNodes.get(nodes)
    if (!anchors) {
        anchors = placeRouteLabels(edges, nodes)
        byNodes.set(nodes, anchors)
    }
    return anchors
}

// A route's label names a choice (a human gate's answers); routes without one stay unlabelled.
function edgeRouteLabel(data: unknown): string {
    const label = (data as Record<string, unknown> | undefined)?.label
    return typeof label === 'string' ? label.trim() : ''
}

function buildFallbackRoute(sourceX: number, sourceY: number, targetX: number, targetY: number): EdgeRoute {
    return normalizeRoute([
        { x: sourceX, y: sourceY },
        { x: targetX, y: targetY },
    ])
}

export function ValidationEdge({
    id,
    source,
    target,
    data,
    markerStart,
    markerEnd,
    style,
    selected,
    sourceX,
    sourceY,
    targetX,
    targetY,
}: EdgeProps) {
    const canvasMode = useCanvasSessionMode()
    const runId = useCanvasRunId()
    const { setEdges, setNodes } = useReactFlow()
    const edgeDiagnostics = useAppStore((state) =>
        canvasMode === 'editor'
            ? state.edgeDiagnostics
            : (runId ? state.runDetailSessionsByRunId[runId]?.edgeDiagnostics ?? EMPTY_DIAGNOSTICS : EMPTY_DIAGNOSTICS),
    )
    const setSelectedNodeId = useAppStore((state) => state.setSelectedNodeId)
    const setSelectedEdgeId = useAppStore((state) => state.setSelectedEdgeId)
    const diagnosticsForEdge = edgeDiagnostics[`${source}->${target}`] || []
    const hasError = diagnosticsForEdge.some((diag) => diag.severity === 'error')
    const hasWarning = diagnosticsForEdge.some((diag) => diag.severity === 'warning')
    const hasInfo = diagnosticsForEdge.some((diag) => diag.severity === 'info')
    const renderRoute = readRenderRoute((data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY])
        ?? buildFallbackRoute(sourceX, sourceY, targetX, targetY)
    const edgePath = buildRoundedPolylinePath(renderRoute)
    const labelPoint = getRouteMidpoint(renderRoute)
    const routeLabel = edgeRouteLabel(data)
    // Only a labelled edge reads the other routes, to keep its label off them and their labels.
    const flowEdges = useFlowStore((state) => (routeLabel ? state.edges : null))
    const flowNodes = useFlowStore((state) => (routeLabel ? state.nodes : null))
    const labelAnchor = (routeLabel && flowEdges && flowNodes ? routeLabelAnchors(flowEdges, flowNodes).get(id) : null)
        ?? (routeLabel ? routeLabelAnchor(renderRoute, [], [], labelWidth(routeLabel)) : labelPoint)
    const derivedPreviewMeta = getDerivedPreviewMeta(data)
    const isDerivedChildEdge = derivedPreviewMeta?.kind === 'child-edge'
    const isDerivedLinkEdge = derivedPreviewMeta?.kind === 'child-link'
    const isEditorSelectable = canvasMode === 'editor' && derivedPreviewMeta?.readOnly !== true

    const handleSelect = useCallback((event: ReactMouseEvent<SVGElement | HTMLDivElement>) => {
        if (!isEditorSelectable) {
            return
        }
        event.stopPropagation()
        setSelectedNodeId(null)
        setSelectedEdgeId(id)
        setNodes((nodes) => nodes.map((node) => (node.selected ? { ...node, selected: false } : node)))
        setEdges((edges) =>
            edges.map((edge) => ({
                ...edge,
                selected: edge.id === id,
            })),
        )
    }, [id, isEditorSelectable, setEdges, setNodes, setSelectedEdgeId, setSelectedNodeId])

    const edgeStyle: CSSProperties = {
        strokeLinecap: 'round',
        strokeLinejoin: 'round',
        strokeWidth: 1.5,
        ...style,
    }

    if (hasError) {
        edgeStyle.stroke = 'hsl(var(--destructive))'
        edgeStyle.strokeWidth = selected ? 2.5 : 1.5
        edgeStyle.opacity = 1
    } else if (hasWarning) {
        edgeStyle.stroke = 'hsl(var(--warning))'
        edgeStyle.strokeWidth = selected ? 2.5 : 1.5
        edgeStyle.opacity = 0.95
    } else if (isDerivedLinkEdge) {
        // Dashed sets the parent-to-child link apart; it stays as legible as any edge (3:1 or better).
        edgeStyle.stroke = 'hsl(var(--muted-foreground) / 0.75)'
        edgeStyle.strokeDasharray = '6 6'
        edgeStyle.strokeWidth = 1.5
    } else if (isDerivedChildEdge) {
        // The child flow's nodes say they are a read-only preview; its edges must still be easy to follow.
        edgeStyle.stroke = 'hsl(var(--muted-foreground) / 0.75)'
        edgeStyle.strokeWidth = 1.5
    } else if (selected) {
        edgeStyle.stroke = 'hsl(var(--primary))'
        edgeStyle.strokeWidth = 2
        edgeStyle.opacity = 1
    }

    const badgeClass = hasError
        ? 'text-destructive'
        : hasWarning
            ? 'text-warning'
            : 'text-info'

    return (
        <>
            <g onClick={handleSelect}>
                <path
                    d={edgePath}
                    fill="none"
                    stroke="transparent"
                    strokeWidth={16}
                    className="react-flow__edge-interaction"
                />
                <BaseEdge
                    id={id}
                    path={edgePath}
                    markerStart={markerStart}
                    markerEnd={markerEnd}
                    style={edgeStyle}
                    interactionWidth={16}
                />
            </g>
            {routeLabel || diagnosticsForEdge.length > 0 ? (
                <EdgeLabelRenderer>
                    <div
                        className="nodrag nopan absolute flex flex-col items-center gap-1"
                        style={{ transform: `translate(-50%, -50%) translate(${labelAnchor.x}px, ${labelAnchor.y}px)` }}
                    >
                        {routeLabel ? (
                            <span
                                data-testid="edge-route-label"
                                title={routeLabel}
                                className="line-clamp-2 max-w-44 break-words rounded-md border border-border bg-background px-1.5 py-px text-center text-xs font-medium leading-snug text-muted-foreground"
                            >
                                {routeLabel}
                            </span>
                        ) : null}
                        {diagnosticsForEdge.length > 0 ? (
                            <div
                                data-testid="edge-diagnostic-badge"
                                onClick={handleSelect}
                                className={`rounded-full border border-border bg-background/95 px-2 py-0.5 text-xs font-medium ${badgeClass}`}
                                title={diagnosticsForEdge.map((diag) => diag.message).join('\n')}
                            >
                                {diagnosticsForEdge.length} {hasError ? 'Error' : hasWarning ? 'Warn' : hasInfo ? 'Info' : 'Issue'}
                            </div>
                        ) : null}
                    </div>
                </EdgeLabelRenderer>
            ) : null}
        </>
    )
}
