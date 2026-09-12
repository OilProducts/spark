import { useStore } from '@/store'
import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { loadAndMigrateClientPreferences, LEGACY_LAYOUT_PREFIX, LAYOUT_CACHE_PREFIX } from '@/features/settings/services/clientPreferencesMigration'
import type { CanvasViewportState } from '@/state/store-types'

import type { EdgeRoute, RouteSide } from './edgeRouting'

export type FlowCanvasKind = 'editor-parent-only' | 'editor-expanded-preview' | 'execution' | 'runs'

export type SavedFlowLayoutEdgeV1 = {
    sourceSide: RouteSide
    targetSide: RouteSide
    sourceSlot: number
    targetSlot: number
    route: EdgeRoute
}

export type SavedFlowLayoutV1 = {
    version: 1
    topologyStamp: string
    nodePositions: Record<string, { x: number; y: number }>
    edgeLayouts: Record<string, SavedFlowLayoutEdgeV1>
    viewport?: CanvasViewportState | null
}

const SAVED_FLOW_LAYOUT_STORAGE_PREFIX = LEGACY_LAYOUT_PREFIX

function isRouteSide(value: unknown): value is RouteSide {
    return value === 'top' || value === 'right' || value === 'bottom' || value === 'left'
}

function isFiniteCoordinate(value: unknown): value is number {
    return Number.isFinite(value)
}

function normalizeEdgeRoute(route: unknown): EdgeRoute | null {
    if (!Array.isArray(route)) {
        return null
    }

    const normalized = route
        .map((point) => {
            if (
                !point
                || typeof point !== 'object'
                || !isFiniteCoordinate((point as { x?: unknown }).x)
                || !isFiniteCoordinate((point as { y?: unknown }).y)
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

function normalizeSavedLayout(raw: unknown): SavedFlowLayoutV1 | null {
    if (!raw || typeof raw !== 'object') {
        return null
    }

    const record = raw as Record<string, unknown>
    if (record.version !== 1 || typeof record.topologyStamp !== 'string') {
        return null
    }

    const nodePositionsRecord = record.nodePositions
    const edgeLayoutsRecord = record.edgeLayouts
    const viewportRecord = record.viewport

    const nodePositions = Object.fromEntries(
        Object.entries(nodePositionsRecord && typeof nodePositionsRecord === 'object'
            ? nodePositionsRecord as Record<string, unknown>
            : {})
            .flatMap(([nodeId, positionValue]) => {
                if (!positionValue || typeof positionValue !== 'object') {
                    return []
                }
                const position = positionValue as { x?: unknown; y?: unknown }
                if (!isFiniteCoordinate(position.x) || !isFiniteCoordinate(position.y)) {
                    return []
                }
                return [[nodeId, { x: position.x, y: position.y }] as const]
            }),
    )

    const edgeLayouts = Object.fromEntries(
        Object.entries(edgeLayoutsRecord && typeof edgeLayoutsRecord === 'object'
            ? edgeLayoutsRecord as Record<string, unknown>
            : {})
            .flatMap(([layoutKey, layoutValue]) => {
                if (!layoutValue || typeof layoutValue !== 'object') {
                    return []
                }
                const layout = layoutValue as Record<string, unknown>
                const route = normalizeEdgeRoute(layout.route)
                if (
                    !isRouteSide(layout.sourceSide)
                    || !isRouteSide(layout.targetSide)
                    || !isFiniteCoordinate(layout.sourceSlot)
                    || !isFiniteCoordinate(layout.targetSlot)
                    || !route
                ) {
                    return []
                }

                return [[layoutKey, {
                    sourceSide: layout.sourceSide,
                    targetSide: layout.targetSide,
                    sourceSlot: Math.max(0, Math.floor(layout.sourceSlot)),
                    targetSlot: Math.max(0, Math.floor(layout.targetSlot)),
                    route,
                } satisfies SavedFlowLayoutEdgeV1] as const]
            }),
    )

    const viewport = viewportRecord && typeof viewportRecord === 'object'
        && isFiniteCoordinate((viewportRecord as { x?: unknown }).x)
        && isFiniteCoordinate((viewportRecord as { y?: unknown }).y)
        && isFiniteCoordinate((viewportRecord as { zoom?: unknown }).zoom)
        ? {
            x: (viewportRecord as { x: number }).x,
            y: (viewportRecord as { y: number }).y,
            zoom: (viewportRecord as { zoom: number }).zoom,
        }
        : undefined

    return {
        version: 1,
        topologyStamp: record.topologyStamp,
        nodePositions,
        edgeLayouts,
        viewport,
    }
}

export function buildSavedFlowLayoutStorageKey(
    projectPath: string | null,
    flowName: string,
    canvasKind: FlowCanvasKind,
): string {
    const normalizedProjectPath = (projectPath ?? '__workspace__').trim() || '__workspace__'
    return `${SAVED_FLOW_LAYOUT_STORAGE_PREFIX}${normalizedProjectPath}:${flowName}:${canvasKind}`
}

export async function loadSavedFlowLayout(
    projectPath: string | null,
    flowName: string,
    canvasKind: FlowCanvasKind,
): Promise<SavedFlowLayoutV1 | null> {
    if (typeof window === 'undefined') {
        return null
    }

    const legacyKey = buildSavedFlowLayoutStorageKey(projectPath, flowName, canvasKind)
    const key = legacyKey.slice(LEGACY_LAYOUT_PREFIX.length)
    if (!useStore.getState().clientPreferencesLoaded && !useStore.getState().clientFlowNodePositions[key]) {
        try {
            const view = await loadAndMigrateClientPreferences()
            useStore.setState({ clientFlowNodePositions: view.effective.flow_node_positions ?? {}, clientFlowEdgePorts: view.effective.flow_edge_ports ?? {} })
        } catch {
            // The controller reports migration/load failures. Preserve accessible legacy data.
            const raw = window.localStorage.getItem(legacyKey)
            if (raw) {
                try {
                    const layout = normalizeSavedLayout(JSON.parse(raw))
                    if (layout) useStore.setState((state) => ({ clientFlowNodePositions: { ...state.clientFlowNodePositions, [key]: layout.nodePositions } }))
                    return layout
                } catch { return null }
            }
        }
    }
    const nodePositions = useStore.getState().clientFlowNodePositions[key]
    if (!nodePositions) return null
    let cache = {}
    try { cache = JSON.parse(window.localStorage.getItem(`${LAYOUT_CACHE_PREFIX}${key}`) ?? '{}') } catch { /* Recompute corrupt caches. */ }
    const layout = normalizeSavedLayout({ version: 1, topologyStamp: '', edgeLayouts: {}, ...cache, nodePositions })
    if (!layout) return null
    const ports = useStore.getState().clientFlowEdgePorts[key]
    if (ports) layout.edgeLayouts = Object.fromEntries(Object.entries(ports).map(([id, edge]) => [id, {
        sourceSide: edge.source_side, targetSide: edge.target_side, sourceSlot: edge.source_slot, targetSlot: edge.target_slot,
        route: layout.edgeLayouts[id]?.route ?? [],
    }]))
    return layout

}

export function saveSavedFlowLayout(
    projectPath: string | null,
    flowName: string,
    canvasKind: FlowCanvasKind,
    layout: SavedFlowLayoutV1,
    userControlled = true,
): void {
    if (typeof window === 'undefined') {
        return
    }

    const key = buildSavedFlowLayoutStorageKey(projectPath, flowName, canvasKind).slice(LEGACY_LAYOUT_PREFIX.length)
    if (userControlled) {
        const positions = { ...useStore.getState().clientFlowNodePositions, [key]: layout.nodePositions }
        const ports = { ...useStore.getState().clientFlowEdgePorts, [key]: Object.fromEntries(Object.entries(layout.edgeLayouts).map(([id, edge]) => [id, {
            source_side: edge.sourceSide, target_side: edge.targetSide, source_slot: edge.sourceSlot, target_slot: edge.targetSlot,
        }])) }
        useStore.setState({ clientFlowNodePositions: positions, clientFlowEdgePorts: ports })
        completePreferenceInteraction({ flow_node_positions: positions, flow_edge_ports: ports })
    }
    const cache = { version: layout.version, topologyStamp: layout.topologyStamp, edgeLayouts: layout.edgeLayouts, viewport: layout.viewport }
    try { window.localStorage.setItem(`${LAYOUT_CACHE_PREFIX}${key}`, JSON.stringify(cache)) } catch { /* Recomputable cache. */ }

}

export function clearSavedFlowLayout(
    projectPath: string | null,
    flowName: string,
    canvasKind: FlowCanvasKind,
): void {
    if (typeof window === 'undefined') {
        return
    }

    const legacyKey = buildSavedFlowLayoutStorageKey(projectPath, flowName, canvasKind)
    const key = legacyKey.slice(LEGACY_LAYOUT_PREFIX.length)
    const positions = { ...useStore.getState().clientFlowNodePositions }
    delete positions[key]
    const ports = { ...useStore.getState().clientFlowEdgePorts }
    delete ports[key]
    useStore.setState({ clientFlowNodePositions: positions, clientFlowEdgePorts: ports })
    completePreferenceInteraction({ flow_node_positions: positions, flow_edge_ports: ports })
    try { window.localStorage.removeItem(`${LAYOUT_CACHE_PREFIX}${key}`) } catch { /* Recomputable cache. */ }
}
