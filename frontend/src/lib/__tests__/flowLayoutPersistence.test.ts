import { afterEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { fetchClientPreferences, type ClientPreferencesView } from '@/features/settings/services/clientPreferences'
import { loadSavedFlowLayout, saveSavedFlowLayout } from '@/lib/flowLayoutPersistence'

vi.mock('@/features/settings/services/clientPreferences', async (original) => ({ ...await original<typeof import('@/features/settings/services/clientPreferences')>(), fetchClientPreferences: vi.fn() }))
const initial = useStore.getState()
afterEach(() => { localStorage.clear(); useStore.setState(initial, true); vi.resetAllMocks() })
const key = '/project:flow:editor-parent-only'
const layout = { version: 1 as const, topologyStamp: 'cached-topology', nodePositions: { a: { x: 12, y: 34 } }, edgeLayouts: {}, viewport: { x: 1, y: 2, zoom: 1 } }

it('completed layout interactions write only user positions and ports to preferences', () => {
    const completed = vi.fn()
    window.addEventListener('spark:preferences-completed', completed)
    try {
        saveSavedFlowLayout('/project', 'flow', 'editor-parent-only', layout)
        expect(completed.mock.calls[0][0].detail).toEqual({ flow_node_positions: { [key]: { a: { x: 12, y: 34 } } }, flow_edge_ports: { [key]: {} } })
        const cache = JSON.parse(localStorage.getItem(`spark.flow_layout_cache.v1:${key}`)!)
        expect(cache.topologyStamp).toBe('cached-topology')
        expect(cache.nodePositions).toBeUndefined()
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
})

it('restores positions and edge ports from server preferences without a local cache', async () => {
    const ports = { edge: { source_side: 'bottom' as const, target_side: 'top' as const, source_slot: 0, target_slot: 0 } }
    const view: ClientPreferencesView = { client_id: 'desktop-stable', revision: 'one', stored: { editor_mode: null, editor_sidebar_width: 444 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 444, flow_node_positions: { [key]: { a: { x: 12, y: 34 } } }, flow_edge_ports: { [key]: ports } } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    const restored = await loadSavedFlowLayout('/project', 'flow', 'editor-parent-only')
    expect(restored?.nodePositions).toEqual({ a: { x: 12, y: 34 } })
    expect(restored?.topologyStamp).toBe('')
    expect(restored?.edgeLayouts.edge).toEqual({ sourceSide: 'bottom', targetSide: 'top', sourceSlot: 0, targetSlot: 0, route: [] })
})
