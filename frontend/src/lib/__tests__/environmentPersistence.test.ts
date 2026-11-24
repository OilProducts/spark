import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { markAttentionSeen, readAttentionWatermark } from '@/features/overview/model/overviewModel'
import { markChatRead, readChatReadState } from '@/features/projects/model/chatReadState'
import { fetchClientPreferences, type ClientPreferencesView } from '@/features/settings/services/clientPreferences'
import type { AttentionItem } from '@/lib/api/attentionApi'
import { loadSavedFlowLayout, saveSavedFlowLayout } from '@/lib/flowLayoutPersistence'
import { useStore } from '@/store'
import { deferred, switchEnvironment } from '@/test/environments'

vi.mock('@/features/settings/services/clientPreferences', async (original) => ({
    ...await original<typeof import('@/features/settings/services/clientPreferences')>(),
    fetchClientPreferences: vi.fn(),
}))

// Each environment's browser state lives under its own keys: the same names in another never touch it.
beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
})

afterEach(() => {
    vi.useRealTimers()
    vi.resetAllMocks()
})

it('keeps the Overview watermarks per environment', async () => {
    const seen = (updatedAt: string) => markAttentionSeen([{ updated_at: updatedAt } as AttentionItem])
    seen('2026-11-01T00:00:00Z')
    await switchEnvironment('env-r')
    expect(readAttentionWatermark()).toBeNull()
    seen('2026-11-20T00:00:00Z')

    await switchEnvironment(null)
    expect(readAttentionWatermark()).toBe(Date.parse('2026-11-01T00:00:00Z'))
    await switchEnvironment('env-r')
    expect(readAttentionWatermark()).toBe(Date.parse('2026-11-20T00:00:00Z'))
})

it('caches flow layouts per environment, and loads positions only into the environment that asked', async () => {
    const layout = (topologyStamp: string) => ({ version: 1 as const, topologyStamp, nodePositions: { a: { x: 1, y: 2 } }, edgeLayouts: {} })
    const showPositions = () => useStore.setState({ clientPreferencesLoaded: true, clientFlowNodePositions: { '/project:flow:editor-parent-only': { a: { x: 1, y: 2 } } } })
    saveSavedFlowLayout('/project', 'flow', 'editor-parent-only', layout('local'), false)
    await switchEnvironment('env-r')
    saveSavedFlowLayout('/project', 'flow', 'editor-parent-only', layout('remote'), false)
    showPositions()
    expect((await loadSavedFlowLayout('/project', 'flow', 'editor-parent-only'))?.topologyStamp).toBe('remote')

    await switchEnvironment(null)
    showPositions()
    expect((await loadSavedFlowLayout('/project', 'flow', 'editor-parent-only'))?.topologyStamp).toBe('local')

    // Preferences that answer after a switch are the environment left.
    await switchEnvironment('env-r')
    useStore.setState(useStore.getInitialState(), true)
    const preferences = deferred<ClientPreferencesView>()
    vi.mocked(fetchClientPreferences).mockReturnValue(preferences.promise)
    const loading = loadSavedFlowLayout('/project', 'flow', 'editor-parent-only')
    await switchEnvironment(null)
    preferences.resolve({ client_id: 'browser-one', revision: 'one', stored: { editor_mode: null, editor_sidebar_width: null },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, flow_node_positions: { '/project:flow:editor-parent-only': { a: { x: 9, y: 9 } } } } })
    await expect(loading).resolves.toBeNull()
    expect(fetchClientPreferences).toHaveBeenCalledWith('env-r')
    expect(useStore.getState().clientFlowNodePositions).toEqual({})
})

it('keeps chat read state per environment', async () => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(1_000)
    markChatRead('conversation-1', '5')
    await switchEnvironment('env-r')
    vi.setSystemTime(2_000)
    expect(readChatReadState()).toEqual({ since: 2_000, revisions: {} })
    markChatRead('conversation-1', '9')

    await switchEnvironment(null)
    expect(readChatReadState()).toEqual({ since: 1_000, revisions: { 'conversation-1': '5' } })
    await switchEnvironment('env-r')
    expect(readChatReadState()).toEqual({ since: 2_000, revisions: { 'conversation-1': '9' } })
})
