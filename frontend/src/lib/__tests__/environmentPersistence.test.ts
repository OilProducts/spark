import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { markAttentionSeen, markFinishedSeen, readAttentionWatermark, readFinishedWatermark } from '@/features/overview/model/overviewModel'
import { markChatRead, readChatReadState } from '@/features/projects/model/chatReadState'
import { fetchClientPreferences, type ClientPreferencesView } from '@/features/settings/services/clientPreferences'
import type { AttentionItem } from '@/lib/api/attentionApi'
import type { RunRecord } from '@/features/runs/model/shared'
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
    const seen = (at: string) => {
        markAttentionSeen([{ updated_at: at } as AttentionItem])
        markFinishedSeen([{ run_id: 'run-1', parent_run_id: null, status: 'completed', outcome: 'success', ended_at: at } as RunRecord], [])
    }
    const watermarks = () => [readAttentionWatermark(), readFinishedWatermark()]
    seen('2026-11-01T00:00:00Z')
    await switchEnvironment('env-r')
    expect(watermarks()).toEqual([null, null])
    seen('2026-11-20T00:00:00Z')

    await switchEnvironment(null)
    expect(watermarks()).toEqual([Date.parse('2026-11-01T00:00:00Z'), Date.parse('2026-11-01T00:00:00Z')])
    await switchEnvironment('env-r')
    expect(watermarks()).toEqual([Date.parse('2026-11-20T00:00:00Z'), Date.parse('2026-11-20T00:00:00Z')])
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

it("keeps an environment's chat read state steady when storage refuses writes", async () => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(1_000)
    const local = readChatReadState()
    const setItem = vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => { throw new DOMException('Full', 'QuotaExceededError') })
    try {
        // A new environment's first read cannot be stored; its snapshot is still one object.
        await switchEnvironment('env-full')
        vi.setSystemTime(2_000)
        const fresh = readChatReadState()
        vi.setSystemTime(3_000)
        expect(readChatReadState()).toBe(fresh)
        expect(fresh).toEqual({ since: 2_000, revisions: {} })

        markChatRead('conversation-1', '9')
        const read = readChatReadState()
        expect(read).toEqual({ since: 2_000, revisions: { 'conversation-1': '9' } })
        expect(readChatReadState()).toBe(read)

        await switchEnvironment(null)
        expect(readChatReadState()).toBe(local)
    } finally {
        setItem.mockRestore()
    }
})
