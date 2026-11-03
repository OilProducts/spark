import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { TEST_GENERATION, conversationPage } from '@/test/conversationWire'
import { useConversationStream } from '../useConversationStream'
import { useProjectConversationCache } from '../useProjectConversationCache'

const chat = {
    conversation_id: 'chat', project_path: '/p', title: 'Chat', revision: 3,
    created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z',
    turns: [{ id: 'turn-1', role: 'user', content: 'Hi', status: 'complete', kind: 'message', timestamp: '2026-11-01T00:00:00Z' }],
}
const emptyWindow = {
    generation: TEST_GENERATION, watermark: '7', conversation_id: 'chat', rows: [],
    turn_ids: [], pinned_turn_ids: [], pinned_artifact_ids: [], older_cursor: null,
}
const reset = { code: 'resync_required', reason: 'catchup_limit', generation: TEST_GENERATION, watermark: '7' }
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), {
    status, headers: { 'content-type': 'application/json' },
})
const notFound = () => json(404, { detail: 'Unknown conversation: chat' })

let fetchMock: ReturnType<typeof vi.fn>

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

function live(type: string, payload: unknown) {
    act(() => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', {
            detail: { type, conversationId: 'chat', projectPath: '/p', payload },
        }))
    })
}

function openChat() {
    const cache = renderHook(() => useProjectConversationCache({
        persistProjectState: async () => {},
        projectSessionsByPath: {},
        updateProjectSessionState: () => {},
    }))
    renderHook(() => useConversationStream({
        activeConversationId: 'chat',
        activeProjectPath: '/p',
        getConversationRecord: (id) => useStore.getState().homeConversationCache.conversationsById[id],
        applyConversationPage: (...args) => cache.result.current.applyConversationPage(...args),
        applyConversationBatch: (...args) => cache.result.current.applyConversationBatch(...args),
        applyTransientConversationEvent: (...args) => cache.result.current.applyTransientConversationEvent(...args),
        resetConversation: (id) => cache.result.current.resetConversation(id),
        markConversationUnavailable: (id) => cache.result.current.markConversationUnavailable(id),
        formatErrorMessage: (_error, fallback) => fallback,
        setPanelError: () => {},
    }))
}

const cache = () => useStore.getState().homeConversationCache

/** A page request after a reset that answers only when the test says so. */
function deferredReload() {
    let answer!: (response: Response) => void
    fetchMock.mockImplementationOnce(() => new Promise<Response>((resolve) => { answer = resolve }))
    return (response: Response) => answer(response)
}

async function opened() {
    fetchMock.mockResolvedValueOnce(json(200, conversationPage(chat)))
    openChat()
    await waitFor(() => expect(cache().conversationsById.chat).toBeDefined())
}

it('a reset, then an empty window, then a 404 shows the chat as unavailable', async () => {
    await opened()
    const answer = deferredReload()
    live('conversation.reset', reset)
    // The server's window for a conversation it no longer has is empty.
    live('conversation.window', emptyWindow)
    await act(async () => { answer(notFound()) })
    await waitFor(() => expect(cache().unavailableConversationIds?.chat).toBe(true))
    expect(cache().conversationsById.chat).toBeUndefined()
})

it('a reset, then a 404, then an empty window keeps the chat unavailable', async () => {
    await opened()
    const answer = deferredReload()
    live('conversation.reset', reset)
    await act(async () => { answer(notFound()) })
    await waitFor(() => expect(cache().unavailableConversationIds?.chat).toBe(true))
    live('conversation.window', emptyWindow)
    expect(cache().unavailableConversationIds?.chat).toBe(true)
    expect(cache().conversationsById.chat).toBeUndefined()
})

it('a new thread is a 404 and an empty window, never unavailable', async () => {
    fetchMock.mockResolvedValueOnce(notFound())
    openChat()
    live('conversation.window', emptyWindow)
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))
    await act(async () => {})
    expect(cache().conversationsById.chat).toBeDefined()
    expect(cache().unavailableConversationIds?.chat).toBeUndefined()
})

it('a newer stream window, even from a new store generation, survives a page request still in flight', async () => {
    const answer = deferredReload()
    openChat()
    const fresh = conversationPage({ ...chat, title: 'New store' }, { generation: 'generation-new', revision: '2' })
    live('conversation.window', fresh)
    expect(cache().conversationsById.chat.generation).toBe('generation-new')
    // The page the hook asked for first lands late, from the old store.
    await act(async () => { answer(json(200, conversationPage({ ...chat, title: 'Old store' }, { revision: '50' }))) })
    expect(cache().conversationsById.chat.generation).toBe('generation-new')
    expect(cache().conversationsById.chat.watermark).toBe('2')
    // Live batches go on applying on top of the stream's window.
    live('conversation.changes', {
        generation: 'generation-new', from_revision: '2', watermark: '3',
        rows: [{ kind: 'turn', conversation_id: 'chat', id: 'turn-2', position: '2', revision: '3', deleted: false,
            payload: { id: 'turn-2', role: 'user', content: 'Later', status: 'complete', kind: 'message', timestamp: '2026-11-01T00:00:01Z' } }],
    })
    expect(cache().conversationsById.chat.watermark).toBe('3')
    expect(Object.keys(cache().conversationsById.chat.rows).some((key) => key.includes('turn-2'))).toBe(true)
})
