import { afterEach, expect, it, vi } from 'vitest'
import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { useStore } from '@/store'
import { fetchClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { loadAndMigrateClientPreferences, LEGACY_LAYOUT_PREFIX, LAYOUT_CACHE_PREFIX } from '../services/clientPreferencesMigration'
import { loadSavedFlowLayout, saveSavedFlowLayout } from '@/lib/flowLayoutPersistence'

vi.mock('@/lib/api/apiClient', () => ({ fetchWorkspaceJsonValidated: vi.fn() }))
vi.mock('../services/clientPreferences', async (original) => ({ ...await original<typeof import('../services/clientPreferences')>(), fetchClientPreferences: vi.fn() }))
const initial = useStore.getState()
afterEach(() => { localStorage.clear(); useStore.setState(initial, true); vi.restoreAllMocks(); vi.resetAllMocks() })
const key = '/project:flow:editor-parent-only'
const raw = JSON.stringify({ version: 1, topologyStamp: 'cached-topology', nodePositions: { a: { x: 12, y: 34 } }, edgeLayouts: {}, viewport: { x: 1, y: 2, zoom: 1 } })
const view: ClientPreferencesView = { client_id: 'desktop-stable', revision: 'one', stored: { editor_mode: null, editor_sidebar_width: 444 }, effective: { editor_mode: 'structured', editor_sidebar_width: 444 } }
it('backs up before import, separates caches, and does not import twice', async () => {
    localStorage.setItem(`${LEGACY_LAYOUT_PREFIX}${key}`, raw)
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    const migrated = { ...view, browser_migration_version: 1, revision: 'two', effective: { ...view.effective, flow_node_positions: { [key]: { a: { x: 12, y: 34 } } } } }
    vi.mocked(fetchWorkspaceJsonValidated).mockImplementation(async () => {
        expect(localStorage.getItem(`${LEGACY_LAYOUT_PREFIX}${key}.v0.bak`)).toBe(raw)
        return migrated
    })
    expect(await loadAndMigrateClientPreferences()).toEqual(migrated)
    const options = vi.mocked(fetchWorkspaceJsonValidated).mock.calls[0][1]
    const payload = JSON.parse(options?.body as string)
    expect(payload).toMatchObject({ expected_revision: 'one', section: 'import_client_preferences', value: { client_id: 'desktop-stable' } })
    expect(payload.value.preferences.flow_node_positions[key]).toEqual({ a: { x: 12, y: 34 } })
    expect(JSON.stringify(payload)).not.toContain('topologyStamp')
    expect(localStorage.getItem(`${LEGACY_LAYOUT_PREFIX}${key}`)).toBeNull()
    const cache = JSON.parse(localStorage.getItem(`${LAYOUT_CACHE_PREFIX}${key}`)!)
    expect(cache.topologyStamp).toBe('cached-topology')
    expect(cache.nodePositions).toBeUndefined()
    vi.mocked(fetchClientPreferences).mockResolvedValue(migrated)
    await loadAndMigrateClientPreferences()
    expect(fetchWorkspaceJsonValidated).toHaveBeenCalledTimes(1)
    localStorage.clear() // A restarted Desktop server uses a different origin/port.
    const restored = await loadSavedFlowLayout('/project', 'flow', 'editor-parent-only')
    expect(restored?.nodePositions).toEqual({ a: { x: 12, y: 34 } })
    expect(restored?.topologyStamp).toBe('')
})

it('retains original data and backup when import conflicts, and refuses to replace a different backup', async () => {
    localStorage.setItem(`${LEGACY_LAYOUT_PREFIX}${key}`, raw)
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(fetchWorkspaceJsonValidated).mockRejectedValue(new Error('Conflict'))
    await expect(loadAndMigrateClientPreferences()).rejects.toThrow('Conflict')
    expect(localStorage.getItem(`${LEGACY_LAYOUT_PREFIX}${key}`)).toBe(raw)
    expect(localStorage.getItem(`${LEGACY_LAYOUT_PREFIX}${key}.v0.bak`)).toBe(raw)
    localStorage.setItem(`${LEGACY_LAYOUT_PREFIX}${key}`, '{}')
    await expect(loadAndMigrateClientPreferences()).rejects.toThrow('backup differs')
    expect(fetchWorkspaceJsonValidated).toHaveBeenCalledTimes(1)
})

it('completed layout interactions write only user positions to preferences', () => {
    const completed = vi.fn()
    window.addEventListener('spark:preferences-completed', completed)
    try {
        saveSavedFlowLayout('/project', 'flow', 'editor-parent-only', JSON.parse(raw))
        expect(completed.mock.calls[0][0].detail).toEqual({ flow_node_positions: { [key]: { a: { x: 12, y: 34 } } }, flow_edge_ports: { [key]: {} } })
        expect(localStorage.getItem(`${LEGACY_LAYOUT_PREFIX}${key}`)).toBeNull()
        expect(JSON.parse(localStorage.getItem(`${LAYOUT_CACHE_PREFIX}${key}`)!).nodePositions).toBeUndefined()
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
})

it('restores user edge ports without storing computed routes in preferences', async () => {
    const edges = { edge: { sourceSide: 'bottom', targetSide: 'top', sourceSlot: 0, targetSlot: 0, route: [{ x: 1, y: 2 }, { x: 3, y: 4 }] } }
    localStorage.setItem(`${LEGACY_LAYOUT_PREFIX}${key}`, JSON.stringify({ ...JSON.parse(raw), edgeLayouts: edges }))
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    const ports = { edge: { source_side: 'bottom' as const, target_side: 'top' as const, source_slot: 0, target_slot: 0 } }
    const migrated = { ...view, browser_migration_version: 1, effective: { ...view.effective, flow_node_positions: { [key]: { a: { x: 12, y: 34 } } }, flow_edge_ports: { [key]: ports } } }
    vi.mocked(fetchWorkspaceJsonValidated).mockResolvedValue(migrated)
    await loadAndMigrateClientPreferences()
    const body = JSON.parse(vi.mocked(fetchWorkspaceJsonValidated).mock.calls[0][1]?.body as string)
    expect(body.value.preferences.flow_edge_ports[key]).toEqual(ports)
    expect(JSON.stringify(body)).not.toContain('route')
    vi.mocked(fetchClientPreferences).mockResolvedValue(migrated)
    localStorage.clear()
    const layout = await loadSavedFlowLayout('/project', 'flow', 'editor-parent-only')
    expect(layout?.edgeLayouts.edge).toEqual({ sourceSide: 'bottom', targetSide: 'top', sourceSlot: 0, targetSlot: 0, route: [] })
})

it('uses authoritative preferences when browser legacy storage is inaccessible', async () => {
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    localStorage.setItem('unrelated', 'operational-cache')
    vi.spyOn(localStorage, 'key').mockImplementation(() => { throw new Error('Storage denied') })
    expect(await loadAndMigrateClientPreferences()).toEqual(view)
    expect(fetchWorkspaceJsonValidated).not.toHaveBeenCalled()
})
