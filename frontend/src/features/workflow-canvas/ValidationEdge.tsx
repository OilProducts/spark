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
import { EDGE_RENDER_ROUTE_KEY, readNodeRect } from '@/lib/flowLayout'
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

// A label's box (leading-snug, px-1.5, py-px and a 1px border), estimated so labels can be laid out
// and placed before they render: 12px text, wrapped past 13.5em to two lines at most, at `scale`.
export function routeLabelSize(text: string, scale = 1): LabelSize {
    const textWidth = text.length * 6.6
    const lines = textWidth > 162 ? 2 : 1
    return { width: Math.min(textWidth, 162) * scale + 14, height: lines * 16.5 * scale + 4 }
}

// How much .canvas-route-label grows at `zoom`, rounded up to a tenth so labels are placed again
// only when it changes by that much.
export const routeLabelScale = (zoom: number) => Math.ceil(Math.min(17, Math.max(12, 10 / zoom)) / 1.2) / 10
export const MAX_ROUTE_LABEL_SCALE = routeLabelScale(0)
// Clear space kept between a label and other routes, labels and nodes, so it reads as its own route's.
const LABEL_CLEARANCE = 6

type LabelSize = { width: number; height: number }
type LabelAnchor = { x: number; y: number }
type LabelBox = { left: number; right: number; top: number; bottom: number }

const labelBox = (anchor: LabelAnchor, size: LabelSize, margin = 0): LabelBox => ({
    left: anchor.x - size.width / 2 - margin,
    right: anchor.x + size.width / 2 + margin,
    top: anchor.y - size.height / 2 - margin,
    bottom: anchor.y + size.height / 2 + margin,
})

const segmentEntersBox = (from: EdgeRoutePoint, to: EdgeRoutePoint, box: LabelBox) => (
    Math.max(from.x, to.x) > box.left + 1 && Math.min(from.x, to.x) < box.right - 1
    && Math.max(from.y, to.y) > box.top + 1 && Math.min(from.y, to.y) < box.bottom - 1
)

/**
 * Where a route's label sits: centred on one of its runs, wherever the label and the space round
 * it meet the fewest other routes, placed labels and nodes. Of the clear spots, the one nearest
 * the route's source wins, in the middle of its run, so a gate's answer reads beside the gate.
 */
export function routeLabelAnchor(
    route: EdgeRoute,
    otherRoutes: EdgeRoute[] = [],
    placedLabels: LabelBox[] = [],
    size: LabelSize = { width: 48, height: 20 },
): LabelAnchor {
    const best: { anchor: LabelAnchor | null; cost: number } = { anchor: null, cost: Infinity }
    const consider = (anchor: LabelAnchor, onSegment: number, preference: number) => {
        const box = labelBox(anchor, size)
        const clearance = labelBox(anchor, size, LABEL_CLEARANCE)
        const routeHits = otherRoutes.reduce((hits, other) => hits + (other.slice(1).some((to, at) => segmentEntersBox(other[at], to, clearance)) ? 1 : 0), 0)
        const ownHits = route.slice(1).filter((to, at) => at !== onSegment && segmentEntersBox(route[at], to, box)).length
        const labelHits = placedLabels.filter((other) => other.left < clearance.right && other.right > clearance.left && other.top < clearance.bottom && other.bottom > clearance.top).length
        // A hit outweighs any preference, so a clear spot always wins.
        const cost = (routeHits + ownHits + 2 * labelHits) * 1e6 + preference
        if (cost < best.cost) Object.assign(best, { anchor, cost })
    }
    let travelled = 0
    route.slice(1).forEach((to, at) => {
        const from = route[at]
        const horizontal = Math.abs(to.y - from.y) < 1
        const [start, end] = horizontal ? [from.x, to.x] : [from.y, to.y]
        const length = Math.abs(end - start)
        const half = (horizontal ? size.width : size.height) / 2
        const middle = (start + end) / 2
        const pointAt = (position: number): LabelAnchor => (horizontal ? { x: position, y: from.y } : { x: from.x, y: position })
        // ponytail: samples every 8px; labels are few, so this stays cheap on the largest flows.
        for (let position = Math.min(start, end) + half; position <= Math.max(start, end) - half + 0.5; position += 8) {
            consider(pointAt(position), at, travelled + Math.abs(position - middle))
        }
        consider(pointAt(middle), at, travelled)
        travelled += length
    })
    return best.anchor ?? getRouteMidpoint(route)
}

/** Every labelled route's label anchor, placed in edge order so each label keeps clear of the nodes and the labels before it. */
function placeRouteLabels(edges: Edge[], nodes: Node[], scale: number): Map<string, LabelAnchor> {
    const routes = edges.flatMap((edge) => {
        const route = edge.hidden ? null : readRenderRoute((edge.data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY])
        return route ? [{ id: edge.id, route, label: edgeRouteLabel(edge.data) }] : []
    })
    // The laid-out size: a run graph's nodes are never measured into its store.
    const placed: LabelBox[] = nodes.flatMap((node) => {
        const rect = readNodeRect(node)
        return node.hidden ? [] : [{ left: rect.x, right: rect.x + rect.width, top: rect.y, bottom: rect.y + rect.height }]
    })
    const anchors = new Map<string, LabelAnchor>()
    for (const { id, route, label } of routes) {
        if (!label) continue
        const size = routeLabelSize(label, scale)
        const anchor = routeLabelAnchor(route, routes.filter((other) => other.id !== id).map((other) => other.route), placed, size)
        anchors.set(id, anchor)
        placed.push(labelBox(anchor, size))
    }
    return anchors
}

// One placement per graph and label scale, shared by every labelled edge that renders from it.
const placedLabelsByGraph = new WeakMap<Edge[], WeakMap<Node[], Map<number, Map<string, LabelAnchor>>>>()
function routeLabelAnchors(edges: Edge[], nodes: Node[], scale: number): Map<string, LabelAnchor> {
    const byNodes = placedLabelsByGraph.get(edges) ?? new WeakMap<Node[], Map<number, Map<string, LabelAnchor>>>()
    placedLabelsByGraph.set(edges, byNodes)
    const byScale = byNodes.get(nodes) ?? new Map<number, Map<string, LabelAnchor>>()
    byNodes.set(nodes, byScale)
    let anchors = byScale.get(scale)
    if (!anchors) {
        anchors = placeRouteLabels(edges, nodes, scale)
        byScale.set(scale, anchors)
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
    const labelScale = useFlowStore((state) => (routeLabel ? routeLabelScale(state.transform[2]) : 1))
    const labelAnchor = (routeLabel && flowEdges && flowNodes ? routeLabelAnchors(flowEdges, flowNodes, labelScale).get(id) : null)
        ?? (routeLabel ? routeLabelAnchor(renderRoute, [], [], routeLabelSize(routeLabel, labelScale)) : labelPoint)
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
                                className="canvas-route-label line-clamp-2 max-w-[calc(13.5em+14px)] break-words rounded-md border border-border bg-background px-1.5 py-px text-center font-medium leading-snug text-muted-foreground"
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
