import type { CSSProperties } from 'react'
import { BaseEdge, EdgeLabelRenderer, type EdgeProps, type InternalNode, useStore as useFlowStore } from '@xyflow/react'

import {
    buildAnchoredOrthogonalRoute,
    buildFallbackOrthogonalRoute,
    buildPolylinePath,
    getRouteMidpoint,
    type EdgeRoute,
    type NodeRect,
    type RouteSide,
} from '@/lib/edgeRouting'
import { useStore as useAppStore } from '@/store'

import { useCanvasSessionMode } from './canvasSessionContext'

const WARNING_STROKE = 'hsl(38 92% 50%)'

function readNodeRect(node?: InternalNode): NodeRect | null {
    if (!node) {
        return null
    }

    const position = node.internals.positionAbsolute
    const width = node.measured.width ?? node.width ?? node.initialWidth ?? 0
    const height = node.measured.height ?? node.height ?? node.initialHeight ?? 0

    if (!Number.isFinite(position.x) || !Number.isFinite(position.y) || width <= 0 || height <= 0) {
        return null
    }

    return {
        x: position.x,
        y: position.y,
        width,
        height,
    }
}

function readLayoutRoute(value: unknown): EdgeRoute | null {
    if (!Array.isArray(value)) {
        return null
    }

    const route = value.filter(
        (point): point is { x: number; y: number } =>
            Boolean(point)
            && typeof point === 'object'
            && Number.isFinite((point as { x?: unknown }).x)
            && Number.isFinite((point as { y?: unknown }).y),
    )
    return route.length >= 2 ? route : null
}

function readRouteSide(value: unknown): RouteSide | null {
    return value === 'top' || value === 'right' || value === 'bottom' || value === 'left'
        ? value
        : null
}

export function ValidationEdge({
    source,
    target,
    data,
    markerEnd,
    style,
    selected,
    sourceX,
    sourceY,
    targetX,
    targetY,
}: EdgeProps) {
    const canvasMode = useCanvasSessionMode()
    const edgeDiagnostics = useAppStore((state) =>
        canvasMode === 'editor'
            ? state.edgeDiagnostics
            : canvasMode === 'runs'
                ? state.runEdgeDiagnostics
                : state.executionEdgeDiagnostics,
    )
    const sourceNode = useFlowStore((state) => state.nodeLookup.get(source))
    const targetNode = useFlowStore((state) => state.nodeLookup.get(target))
    const diagnosticsForEdge = edgeDiagnostics[`${source}->${target}`] || []
    const hasError = diagnosticsForEdge.some((diag) => diag.severity === 'error')
    const hasWarning = diagnosticsForEdge.some((diag) => diag.severity === 'warning')
    const hasInfo = diagnosticsForEdge.some((diag) => diag.severity === 'info')
    const sourceRect = readNodeRect(sourceNode)
    const targetRect = readNodeRect(targetNode)
    const layoutRoute = readLayoutRoute((data as { layoutRoute?: unknown } | undefined)?.layoutRoute)
    const layoutSourceSide = readRouteSide((data as { layoutSourceSide?: unknown } | undefined)?.layoutSourceSide)
    const layoutTargetSide = readRouteSide((data as { layoutTargetSide?: unknown } | undefined)?.layoutTargetSide)
    const liveHintedRoute = sourceRect && targetRect && layoutSourceSide && layoutTargetSide
        ? buildAnchoredOrthogonalRoute(sourceRect, targetRect, layoutSourceSide, layoutTargetSide)
        : null
    const fallbackRoute = sourceRect && targetRect
        ? buildFallbackOrthogonalRoute(sourceRect, targetRect)
        : [
            { x: sourceX, y: sourceY },
            { x: sourceX, y: (sourceY + targetY) / 2 },
            { x: targetX, y: (sourceY + targetY) / 2 },
            { x: targetX, y: targetY },
        ]
    const route = layoutRoute ?? liveHintedRoute ?? fallbackRoute
    const edgePath = buildPolylinePath(route)
    const labelPoint = getRouteMidpoint(route)

    const edgeStyle: CSSProperties = {
        strokeLinecap: 'round',
        strokeLinejoin: 'round',
        ...style,
    }
    if (hasError) {
        edgeStyle.stroke = 'hsl(var(--destructive))'
        edgeStyle.strokeWidth = selected ? 4 : 3
        edgeStyle.opacity = 1
    } else if (hasWarning) {
        edgeStyle.stroke = WARNING_STROKE
        edgeStyle.strokeWidth = selected ? 4 : 3
        edgeStyle.opacity = 0.95
    } else if (selected) {
        edgeStyle.stroke = 'hsl(var(--primary))'
        edgeStyle.strokeWidth = 4
        edgeStyle.opacity = 1
    }

    if (selected) {
        const shadowColor = hasError
            ? 'hsl(var(--destructive) / 0.55)'
            : hasWarning
                ? 'hsl(38 92% 50% / 0.4)'
                : 'hsl(var(--primary) / 0.6)'
        edgeStyle.filter = `drop-shadow(0 0 7px ${shadowColor})`
    }

    const badgeClass = hasError
        ? 'bg-destructive/15 text-destructive'
        : hasWarning
            ? 'bg-amber-500/15 text-amber-800'
            : 'bg-sky-500/15 text-sky-700'

    return (
        <>
            <BaseEdge
                path={edgePath}
                markerEnd={markerEnd}
                style={edgeStyle}
                interactionWidth={16}
                data-testid="validation-edge-path"
            />
            {diagnosticsForEdge.length > 0 && (
                <EdgeLabelRenderer>
                    <div
                        className="nodrag nopan"
                        style={{
                            transform: `translate(-50%, -50%) translate(${labelPoint.x}px, ${labelPoint.y}px)`,
                        }}
                    >
                        <div
                            data-testid="edge-diagnostic-badge"
                            className={`rounded-full border border-border bg-background/95 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${badgeClass}`}
                            title={diagnosticsForEdge.map((diag) => diag.message).join('\n')}
                        >
                            {diagnosticsForEdge.length} {hasError ? 'Error' : hasWarning ? 'Warn' : hasInfo ? 'Info' : 'Issue'}
                        </div>
                    </div>
                </EdgeLabelRenderer>
            )}
        </>
    )
}
