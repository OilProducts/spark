import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import type { ConversationSummaryResponse } from '@/lib/workspaceClient'
import { hydrateConversationRecordFromSnapshot } from '../../model/projectsHomeState'
import { useChatRunning } from '../useChatRunning'
import { useProjectConversationCache } from '../useProjectConversationCache'

const summary = (overrides: Partial<ConversationSummaryResponse> = {}): ConversationSummaryResponse => ({
    conversation_id: 'chat', project_path: '/p', title: 'Chat',
    created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z', revision: 1, running: false,
    ...overrides,
})
const setSummaries = (summariesByProjectPath: Record<string, ConversationSummaryResponse[]>) => useStore.setState({
    homeConversationCache: { ...useStore.getState().homeConversationCache, summariesByProjectPath },
})
let fetchMock: ReturnType<typeof vi.fn>

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

it('lights for a running summary in any project, a send in flight, or the open chat streaming, without fetching', () => {
    setSummaries({ '/p': [summary()], '/q': [summary({ conversation_id: 'other', project_path: '/q' })] })
    const { result } = renderHook(() => useChatRunning())
    expect(result.current).toBe(false)

    act(() => setSummaries({ '/p': [summary()], '/q': [summary({ conversation_id: 'other', project_path: '/q', running: true })] }))
    expect(result.current).toBe(true)
    act(() => setSummaries({}))
    expect(result.current).toBe(false)

    act(() => useStore.getState().setSendingChat('chat', true))
    expect(result.current).toBe(true)
    act(() => useStore.getState().setSendingChat('chat', false))
    expect(result.current).toBe(false)

    // An open chat outside any Threads list (such as a mission's) follows its own turns.
    const record = hydrateConversationRecordFromSnapshot({
        schema_version: 5, revision: 2, conversation_id: 'mission-chat', conversation_handle: '', project_path: '/p',
        chat_mode: 'chat', title: 'Mission', created_at: '', updated_at: '',
        turns: [{ id: 't', role: 'assistant', content: '', timestamp: '', kind: 'message', status: 'streaming', artifact_id: null }],
        segments: [], event_log: [], flow_run_requests: [], flow_launches: [],
    })
    act(() => useStore.setState((state) => ({
        activeProjectPath: '/p',
        projectSessionsByPath: { '/p': { ...state.projectSessionsByPath['/p'], conversationId: 'mission-chat' } },
        homeConversationCache: { ...state.homeConversationCache, conversationsById: { 'mission-chat': record } },
    })))
    expect(result.current).toBe(true)
    expect(fetchMock).not.toHaveBeenCalled()
})

it('keeps a newer upsert that arrived while the startup list was loading', async () => {
    let respond!: (value: unknown) => void
    fetchMock.mockImplementation(() => new Promise((resolve) => {
        respond = (value) => resolve({ ok: true, status: 200, json: async () => value })
    }))
    const cache = renderHook(() => useProjectConversationCache({
        persistProjectState: async () => {},
        projectSessionsByPath: {},
        updateProjectSessionState: () => {},
    }))
    const running = renderHook(() => useChatRunning())
    let loaded!: Promise<ConversationSummaryResponse[]>
    act(() => { loaded = cache.result.current.loadProjectConversationSummaries('/p') })
    act(() => useStore.getState().upsertHomeConversationSummary(summary({ revision: 5, running: true })))
    await act(async () => {
        respond([summary({ revision: 4, running: false }), summary({ conversation_id: 'listed', revision: 1 })])
        await loaded
    })
    expect(running.result.current).toBe(true)
    expect(useStore.getState().homeConversationCache.summariesByProjectPath['/p'].map((entry) => [entry.conversation_id, entry.revision]))
        .toEqual(expect.arrayContaining([['chat', 5], ['listed', 1]]))
})
