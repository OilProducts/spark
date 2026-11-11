const EMPTY_DIAGNOSTICS: Record<string, import('@/store').DiagnosticEntry[]> = {}
import { useCallback, type CSSProperties, type MouseEvent as ReactMouseEvent } from 'react'
import { BaseEdge, EdgeLabelRenderer, useReactFlow, useStore as useFlowStore, type EdgeProps } from '@xyflow/react'

import {
    buildRoundedPolylinePath,
    getRouteMidpoint,
    normalizeRoute,
    routeCrossings,
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

/**
 * Where a route's label sits: on the longest stretch of a horizontal run, in the lane between
 * node rows, that no other route crosses; on a route that only runs vertically, beside its
 * longest run, clear of the line.
 */
export function routeLabelAnchor(route: EdgeRoute, crossings: EdgeRoutePoint[] = []): { x: number; y: number; beside: boolean } {
    let clear: { x: number; y: number; length: number } | null = null
    let horizontal: { x: number; y: number; length: number } | null = null
    let vertical: { x: number; y: number; length: number } | null = null
    for (let index = 1; index < route.length; index += 1) {
        const from = route[index - 1]
        const to = route[index]
        const length = Math.hypot(to.x - from.x, to.y - from.y)
        const point = { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2, length }
        if (Math.abs(to.y - from.y) < 1) {
            if (length >= 48 && length > (horizontal?.length ?? 0)) horizontal = point
            const left = Math.min(from.x, to.x)
            const right = Math.max(from.x, to.x)
            const cuts = [
                left,
                ...crossings.filter((crossing) => Math.abs(crossing.y - from.y) < 1 && crossing.x > left && crossing.x < right).map((crossing) => crossing.x),
                right,
            ].sort((a, b) => a - b)
            for (let cut = 1; cut < cuts.length; cut += 1) {
                const stretch = cuts[cut] - cuts[cut - 1]
                if (stretch >= 48 && stretch > (clear?.length ?? 0)) {
                    clear = { x: (cuts[cut - 1] + cuts[cut]) / 2, y: from.y, length: stretch }
                }
            }
        } else if (length > (vertical?.length ?? 0)) {
            vertical = point
        }
    }
    const best = clear ?? horizontal
    if (best) return { x: best.x, y: best.y, beside: false }
    if (vertical) return { x: vertical.x, y: vertical.y, beside: true }
    return { ...getRouteMidpoint(route), beside: false }
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
    // A route's label names a choice (a human gate's answers); routes without one stay unlabelled.
    const rawLabel = (data as Record<string, unknown> | undefined)?.label
    const routeLabel = typeof rawLabel === 'string' ? rawLabel.trim() : ''
    // Only a labelled edge reads the other routes, to keep its label off their crossings.
    const flowEdges = useFlowStore((state) => (routeLabel ? state.edges : null))
    const crossings = flowEdges
        ? routeCrossings(renderRoute, flowEdges.flatMap((edge) => {
            const route = edge.id === id || edge.hidden ? null : readRenderRoute((edge.data as Record<string, unknown> | undefined)?.[EDGE_RENDER_ROUTE_KEY])
            return route ? [route] : []
        }))
        : []
    const labelAnchor = routeLabel ? routeLabelAnchor(renderRoute, crossings) : { ...labelPoint, beside: false }
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
                        className={`nodrag nopan absolute flex flex-col gap-1 ${labelAnchor.beside ? 'items-end' : 'items-center'}`}
                        style={{
                            transform: labelAnchor.beside
                                ? `translate(-100%, -50%) translate(${labelAnchor.x - 6}px, ${labelAnchor.y}px)`
                                : `translate(-50%, -50%) translate(${labelAnchor.x}px, ${labelAnchor.y}px)`,
                        }}
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
