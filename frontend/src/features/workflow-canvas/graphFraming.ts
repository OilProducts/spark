import { useCallback, useEffect, useRef, type RefObject } from 'react'
import { useReactFlow, type Node, type Viewport } from '@xyflow/react'

import { readNodeRect } from '@/lib/flowLayout'

/** Below this zoom node labels (14px) render under ~11px; a bigger graph opens here and pans. */
export const READABLE_ZOOM = 0.8
const MAX_FRAME_ZOOM = 1

export type FramePadding = { top: number; right: number; bottom: number; left: number }
type Rect = { x: number; y: number; width: number; height: number }

function nodesBounds(nodes: Node[]): Rect {
    const rects = nodes.map(readNodeRect)
    const x = Math.min(...rects.map((rect) => rect.x))
    const y = Math.min(...rects.map((rect) => rect.y))
    return {
        x,
        y,
        width: Math.max(...rects.map((rect) => rect.x + rect.width)) - x,
        height: Math.max(...rects.map((rect) => rect.y + rect.height)) - y,
    }
}

// One axis: centre content that fits; otherwise centre on `focus` but never pull the graph's
// edge inside the padding, so the view shows no empty canvas beyond the graph.
function frameAxis(start: number, size: number, zoom: number, paddingStart: number, available: number, focus: number): number {
    if (size * zoom <= available) {
        return paddingStart + (available - size * zoom) / 2 - start * zoom
    }
    const nearest = paddingStart - start * zoom
    const farthest = paddingStart + available - (start + size) * zoom
    return Math.min(nearest, Math.max(farthest, paddingStart + available / 2 - focus * zoom))
}

/**
 * The viewport that shows the whole graph when it fits at a readable zoom (never magnified past
 * 1×); otherwise the readable zoom around `focus`, or the top of the flow.
 */
export function readableViewport(
    bounds: Rect,
    pane: { width: number; height: number },
    padding: FramePadding,
    focus: { x: number; y: number } | null,
): Viewport {
    const availableWidth = Math.max(1, pane.width - padding.left - padding.right)
    const availableHeight = Math.max(1, pane.height - padding.top - padding.bottom)
    const fitZoom = Math.min(availableWidth / bounds.width, availableHeight / bounds.height, MAX_FRAME_ZOOM)
    const zoom = Math.max(fitZoom, READABLE_ZOOM)
    const center = focus ?? { x: bounds.x + bounds.width / 2, y: bounds.y }
    return {
        x: frameAxis(bounds.x, bounds.width, zoom, padding.left, availableWidth, center.x),
        y: frameAxis(bounds.y, bounds.height, zoom, padding.top, availableHeight, center.y),
        zoom,
    }
}

type FrameRequest = { padding: FramePadding; focusNodeId?: string | null }

/**
 * Frames graphs with `readableViewport`, and frames the graph again when its pane resizes,
 * unless the user has panned or zoomed since.
 */
export function useReadableFraming(paneRef: RefObject<HTMLElement | null>) {
    const { getViewport, setViewport } = useReactFlow()
    const framedRef = useRef<{ nodes: Node[]; request: FrameRequest; viewport: Viewport | null } | null>(null)

    const frame = useCallback((nodes: Node[], request: FrameRequest) => {
        const visibleNodes = nodes.filter((node) => !node.hidden)
        const pane = paneRef.current
        if (visibleNodes.length === 0) {
            return
        }
        // A pane with no size yet (hidden, or not laid out) is framed when it gets one.
        if (!pane || pane.clientWidth === 0 || pane.clientHeight === 0) {
            framedRef.current = { nodes: visibleNodes, request, viewport: null }
            return
        }
        // Without a focus, a graph too big to show whole opens where the flow starts.
        const focusNode = visibleNodes.find((node) => node.id === request.focusNodeId)
            ?? visibleNodes.find((node) => node.data?.kind === 'start')
        const focusRect = focusNode ? readNodeRect(focusNode) : null
        const viewport = readableViewport(
            nodesBounds(visibleNodes),
            { width: pane.clientWidth, height: pane.clientHeight },
            request.padding,
            focusRect ? { x: focusRect.x + focusRect.width / 2, y: focusRect.y + focusRect.height / 2 } : null,
        )
        framedRef.current = { nodes: visibleNodes, request, viewport }
        void setViewport(viewport)
    }, [paneRef, setViewport])

    useEffect(() => {
        const pane = paneRef.current
        if (!pane || typeof ResizeObserver === 'undefined') {
            return
        }
        const observer = new ResizeObserver(() => {
            const framed = framedRef.current
            if (!framed) {
                return
            }
            const current = getViewport()
            const untouched = !framed.viewport || (
                Math.abs(current.x - framed.viewport.x) < 1
                && Math.abs(current.y - framed.viewport.y) < 1
                && Math.abs(current.zoom - framed.viewport.zoom) < 0.001
            )
            if (untouched) {
                frame(framed.nodes, framed.request)
            }
        })
        observer.observe(pane)
        return () => observer.disconnect()
    }, [frame, getViewport, paneRef])

    return frame
}
