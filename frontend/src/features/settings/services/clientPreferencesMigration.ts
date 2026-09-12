import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { fetchClientPreferences, parseClientPreferences, type ClientPreferencesView, type FlowNodePositions, type FlowEdgePorts } from './clientPreferences'

export const LEGACY_LAYOUT_PREFIX = 'spark.saved_flow_layout.v1:'
export const LAYOUT_CACHE_PREFIX = 'spark.flow_layout_cache.v1:'

/** Back up accessible legacy layouts before importing only user-controlled positions. */
export async function loadAndMigrateClientPreferences(): Promise<ClientPreferencesView> {
    let view = await fetchClientPreferences()
    const sources: [string, string][] = []
    const positions: FlowNodePositions = {}
    const ports: FlowEdgePorts = {}
    try {
        for (let index = 0; index < localStorage.length; index++) {
            const key = localStorage.key(index)
            if (!key?.startsWith(LEGACY_LAYOUT_PREFIX) || key.endsWith('.v0.bak')) continue
            const raw = localStorage.getItem(key)
            if (raw === null) continue
            sources.push([key, raw])
        }
    } catch { return view } // Inaccessible browser data cannot block authoritative server preferences.
    for (const [key, raw] of sources) {
        const backup = localStorage.getItem(`${key}.v0.bak`)
        if (backup !== null && backup !== raw) throw new Error('Layout migration backup differs; original browser data is retained.')
        localStorage.setItem(`${key}.v0.bak`, raw)
        let layout: { version: number; nodePositions: Record<string, { x: number; y: number }>; edgeLayouts?: Record<string, { sourceSide: string; targetSide: string; sourceSlot: number; targetSlot: number }> }
        try {
            layout = JSON.parse(raw)
            if (layout.version !== 1 || !layout.nodePositions || typeof layout.nodePositions !== 'object' || Array.isArray(layout.nodePositions)) throw new Error()
            for (const point of Object.values(layout.nodePositions)) {
                if (!point || typeof point.x !== 'number' || !Number.isFinite(point.x) || typeof point.y !== 'number' || !Number.isFinite(point.y)) throw new Error()
            }
        } catch { throw new Error('Invalid legacy layout; original browser data and backup are retained.') }
        positions[key.slice(LEGACY_LAYOUT_PREFIX.length)] = layout.nodePositions
        if (layout.edgeLayouts != null && (typeof layout.edgeLayouts !== 'object' || Array.isArray(layout.edgeLayouts))) throw new Error('Invalid legacy edge ports; original browser data and backup are retained.')
        const edges: FlowEdgePorts[string] = {}
        for (const [id, edge] of Object.entries(layout.edgeLayouts ?? {})) {
            if (![edge?.sourceSide, edge?.targetSide].every((side) => ['top', 'right', 'bottom', 'left'].includes(side)) || ![edge?.sourceSlot, edge?.targetSlot].every((slot) => Number.isInteger(slot) && slot >= 0 && slot <= 4294967295)) throw new Error('Invalid legacy edge ports; original browser data and backup are retained.')
            edges[id] = { source_side: edge.sourceSide as FlowEdgePorts[string][string]['source_side'], target_side: edge.targetSide as FlowEdgePorts[string][string]['target_side'], source_slot: edge.sourceSlot, target_slot: edge.targetSlot }
        }
        ports[key.slice(LEGACY_LAYOUT_PREFIX.length)] = edges
    }
    if (sources.length && !view.browser_migration_version) {
        view = await fetchWorkspaceJsonValidated('/settings', {
            method: 'PATCH', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ expected_revision: view.revision, section: 'import_client_preferences', value: {
                client_id: view.client_id, preferences: { editor_mode: null, editor_sidebar_width: null, flow_node_positions: positions, flow_edge_ports: ports },
            } }),
        }, '/workspace/api/settings', parseClientPreferences)
    }
    for (const [key, raw] of sources) {
        // Cache stays on this origin. Node positions now have one durable authority.
        const cache = JSON.parse(raw) as Record<string, unknown>
        delete cache.nodePositions
        localStorage.setItem(`${LAYOUT_CACHE_PREFIX}${key.slice(LEGACY_LAYOUT_PREFIX.length)}`, JSON.stringify(cache))
        localStorage.removeItem(key)
    }
    return view
}
