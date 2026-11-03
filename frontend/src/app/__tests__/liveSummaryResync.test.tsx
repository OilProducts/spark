import { act, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { applyConversationPageToCache } from '@/features/projects/model/projectsHomeState'
import { parseConversationPageResponse } from '@/lib/workspaceClient'
import {
    TEST_GENERATION,
    conversationBatch,
    conversationPage,
    conversationRow,
    liveEnvelope,
    summarySnapshot,
    tombstoneRow,
} from '@/test/conversationWire'
import { WorkspaceLiveEventsController } from '../AppSessionControllers'

class MockEventSource {
    static instances: MockEventSource[] = []
    onopen: (() => void) | null = null
    onmessage: ((event: MessageEvent) => void) | null = null
    onerror: (() => void) | null = null
    closed = false
    constructor(public url: string) { MockEventSource.instances.push(this) }
    close() { this.closed = true }
    emit(envelope: unknown) {
        this.onmessage?.({ data: JSON.stringify(envelope) } as MessageEvent)
    }
}

const summary = (id: string, projectPath: string, revision: number, title = `Chat ${id}`) => ({
    conversation_id: id, project_path: projectPath, title,
    created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z', revision,
})
const params = (source: MockEventSource) => new URL(source.url, 'http://localhost').searchParams
const latest = () => MockEventSource.instances.at(-1)!
const lists = () => useStore.getState().homeConversationCache.summariesByProjectPath
const listed = (projectPath: string) => (lists()[projectPath] ?? []).map((entry) => [entry.conversation_id, entry.title, entry.revision])
const emit = (envelope: unknown, source = latest()) => act(() => source.emit(envelope))
const snapshotEnvelope = (summaries: ReturnType<typeof summary>[], watermark: string) => liveEnvelope(
    'conversations.snapshot',
    summarySnapshot(summaries, { watermark }),
)
const changesEnvelope = (rows: Record<string, unknown>[], options: { watermark?: string; generation?: string } = {}) => liveEnvelope(
    'conversations.changes',
    conversationBatch(rows, options),
)
const reset = (watermark: string) => ({ code: 'resync_required', reason: 'cursor_ahead', generation: TEST_GENERATION, watermark })
/** Drops the stream and waits for the controller's reconnect after its 500ms delay. */
const reconnect = async () => {
    const count = MockEventSource.instances.length
    act(() => latest().onerror!())
    await waitFor(() => expect(MockEventSource.instances).toHaveLength(count + 1))
    return latest()
}
/** Seeds an open transcript record, with its live watermark, as a loaded window leaves it. */
const seedConversation = (conversationId: string, projectPath: string, revision: number) => useStore.getState().commitHomeConversationCache(
    (current) => applyConversationPageToCache(current, parseConversationPageResponse(conversationPage({
        conversation_id: conversationId, project_path: projectPath, title: `Chat ${conversationId}`, revision,
        created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z',
        turns: [{ id: `${conversationId}-turn`, role: 'user', content: 'Hello', timestamp: '2026-10-28T00:00:00Z', status: 'complete', kind: 'message' }],
        segments: [],
    })), 'window'),
)
let fetchMock: ReturnType<typeof vi.fn>

beforeEach(() => {
    MockEventSource.instances = []
    useStore.setState(useStore.getInitialState(), true)
    vi.stubGlobal('EventSource', MockEventSource)
    fetchMock = vi.fn(() => Promise.resolve({ ok: true, status: 200, json: async () => ({}) }))
    vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => { vi.unstubAllGlobals() })

it('opens the summary feed without a cursor and initializes every project\'s list from its snapshot', () => {
    useStore.setState({
        homeConversationCache: { ...useStore.getState().homeConversationCache, summariesByProjectPath: { '/p': [{ ...summary('stale', '/p', 1), revision: '1' }] } },
    })
    render(<WorkspaceLiveEventsController />)
    const source = latest()
    expect(params(source).get('include_conversations')).toBe('true')
    expect(params(source).has('conversations_project_path')).toBe(false)
    expect(params(source).has('conversations_generation')).toBe(false)
    expect(params(source).has('conversations_revision')).toBe(false)

    // Opening the stream no longer refetches summaries over HTTP; the snapshot is the initial state.
    act(() => source.onopen!())
    emit(snapshotEnvelope([], '2'))
    expect(lists()).toEqual({ '/p': [] })

    emit(snapshotEnvelope([summary('a', '/p', 3), summary('b', '/p', 4), summary('c', '/q', 5)], '6'))
    expect(listed('/p')).toEqual(expect.arrayContaining([['a', 'Chat a', '3'], ['b', 'Chat b', '4']]))
    expect(listed('/p')).toHaveLength(2)
    expect(listed('/q')).toEqual([['c', 'Chat c', '5']])
    expect(useStore.getState().homeConversationCache.summaryCursor).toEqual({ generation: TEST_GENERATION, watermark: '6' })
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes('/projects/conversations'))).toBe(false)
})

it('holds summary changes that arrive before a cursorless connection\'s snapshot and applies them after it', () => {
    render(<WorkspaceLiveEventsController />)
    // A change newer than the snapshot, and one the snapshot already covers.
    emit(changesEnvelope([conversationRow(summary('x', '/p', 7, 'Live'))]))
    emit(changesEnvelope([conversationRow(summary('a', '/p', 3, 'Old'))]))
    expect(lists()).toEqual({})

    emit(snapshotEnvelope([summary('a', '/p', 4, 'New')], '5'))
    expect(listed('/p')).toEqual(expect.arrayContaining([['x', 'Live', '7'], ['a', 'New', '4']]))
    expect(listed('/p')).toHaveLength(2)
    expect(useStore.getState().homeConversationCache.summaryCursor?.watermark).toBe('7')
})

it('reconnects from the latest summary watermark, including one an empty change batch advanced', async () => {
    render(<WorkspaceLiveEventsController />)
    emit(snapshotEnvelope([summary('a', '/p', 3)], '5'))
    let source = await reconnect()
    expect(params(source).get('conversations_generation')).toBe(TEST_GENERATION)
    expect(params(source).get('conversations_revision')).toBe('5')

    // A change made while the stream was closed arrives as catch-up from the cursor.
    emit(changesEnvelope([conversationRow(summary('a', '/p', 6, 'Renamed while closed'))]))
    expect(listed('/p')).toEqual([['a', 'Renamed while closed', '6']])
    emit(changesEnvelope([], { watermark: '9' }))
    source = await reconnect()
    expect(params(source).get('conversations_generation')).toBe(TEST_GENERATION)
    expect(params(source).get('conversations_revision')).toBe('9')
    expect(params(source).get('include_conversations')).toBe('true')
})

it('reconnects without the summary cursor after a summary reset, and the next snapshot drops what it no longer lists', async () => {
    seedConversation('b', '/p', 2)
    render(<WorkspaceLiveEventsController />)
    emit(snapshotEnvelope([summary('a', '/p', 3), summary('b', '/p', 2), summary('c', '/q', 4)], '5'))
    const first = latest()
    emit(liveEnvelope('conversations.reset', reset('5')))
    expect(first.closed).toBe(true)
    const source = latest()
    expect(source).not.toBe(first)
    expect(params(source).get('include_conversations')).toBe('true')
    expect(params(source).has('conversations_generation')).toBe(false)
    expect(params(source).has('conversations_revision')).toBe(false)

    emit(snapshotEnvelope([summary('a', '/p', 3), summary('c', '/q', 4)], '8'))
    expect(listed('/p')).toEqual([['a', 'Chat a', '3']])
    expect(listed('/q')).toEqual([['c', 'Chat c', '4']])
    // The thread list lost the conversation; its open transcript is not the summary feed's to drop.
    expect(useStore.getState().homeConversationCache.conversationsById.b?.orderedTurnIds).toEqual(['b-turn'])
    expect(useStore.getState().homeConversationCache.summaryCursor?.watermark).toBe('8')
})

it('resumes the open chat from its watermark, and reconnects it without one after a conversation reset', () => {
    seedConversation('chat', '/p', 12)
    act(() => {
        useStore.getState().setActiveProjectPath('/p')
        useStore.getState().updateProjectSessionState('/p', { conversationId: 'chat' })
    })
    render(<WorkspaceLiveEventsController />)
    const first = latest()
    expect(params(first).get('conversation_id')).toBe('chat')
    expect(params(first).get('conversation_project_path')).toBe('/p')
    expect(params(first).get('conversation_generation')).toBe(TEST_GENERATION)
    expect(params(first).get('conversation_revision')).toBe('12')

    emit(liveEnvelope('conversation.reset', reset('12'), { conversationId: 'chat', projectPath: '/p' }), first)
    expect(first.closed).toBe(true)
    const source = latest()
    expect(source).not.toBe(first)
    expect(params(source).get('conversation_id')).toBe('chat')
    expect(params(source).get('conversation_project_path')).toBe('/p')
    expect(params(source).has('conversation_generation')).toBe(false)
    expect(params(source).has('conversation_revision')).toBe(false)
})

it('removes a deleted conversation from its thread list', () => {
    render(<WorkspaceLiveEventsController />)
    emit(snapshotEnvelope([summary('a', '/p', 3), summary('b', '/p', 4)], '5'))
    emit(changesEnvelope([tombstoneRow('conversation', 'b', 'b', '6')]))
    expect(listed('/p')).toEqual([['a', 'Chat a', '3']])
    // A row no newer than the tombstone can't bring it back.
    emit(changesEnvelope([conversationRow(summary('b', '/p', 6))], { watermark: '7' }))
    expect(listed('/p')).toEqual([['a', 'Chat a', '3']])
})

it('drops a run journal entry repeated because the server subscribes before reading initial state', () => {
    render(<WorkspaceLiveEventsController />)
    const source = MockEventSource.instances.at(-1)!
    const received: unknown[] = []
    const onEntry = (event: Event) => received.push((event as CustomEvent).detail.entry)
    window.addEventListener('spark:run-journal-entry', onEntry)
    const entry = (sequence: number) => ({ data: JSON.stringify({
        type: 'run.journal_entry', resource: { kind: 'run', id: 'run-1' },
        cursor: { kind: 'run_sequence', value: sequence }, payload: { sequence },
    }) } as MessageEvent)
    act(() => {
        for (const sequence of [1, 2, 2, 3]) source.onmessage!(entry(sequence))
    })
    window.removeEventListener('spark:run-journal-entry', onEntry)
    expect(received).toEqual([{ sequence: 1 }, { sequence: 2 }, { sequence: 3 }])
})

it('a project\'s HTTP thread list loaded before the feed\'s snapshot never starts the feed\'s cursor', async () => {
    const { applySummarySnapshotToCache } = await import('@/features/projects/model/projectsHomeState')
    const { parseConversationSummarySnapshotResponse } = await import('@/lib/workspaceClient')
    render(<WorkspaceLiveEventsController />)
    // Another project already has a conversation at a revision below the filtered watermark.
    useStore.getState().commitHomeConversationCache((current) => applySummarySnapshotToCache(
        current,
        parseConversationSummarySnapshotResponse(summarySnapshot([summary('a', '/a', 40)], { project_path: '/a', watermark: '100' })),
        'http',
        '/a',
    ))
    expect(useStore.getState().homeConversationCache.summaryCursor ?? null).toBeNull()
    // The feed fails before its snapshot; the reconnect still asks for one.
    const source = await reconnect()
    expect(params(source).has('conversations_generation')).toBe(false)
    expect(params(source).has('conversations_revision')).toBe(false)
    emit(snapshotEnvelope([summary('a', '/a', 40), summary('b', '/b', 50)], '100'))
    expect(listed('/b')).toEqual([['b', 'Chat b', '50']])
    expect(useStore.getState().homeConversationCache.summaryCursor).toEqual({ generation: TEST_GENERATION, watermark: '100' })
})

it('after a summary reset, a snapshot from a new store generation replaces every old row and tombstone', () => {
    seedConversation('open', '/p', 2)
    render(<WorkspaceLiveEventsController />)
    emit(snapshotEnvelope([summary('a', '/p', 100, 'OLD'), summary('gone', '/q', 90)], '100'))
    emit(changesEnvelope([tombstoneRow('conversation', 'dead', 'dead', '101')]))
    emit(liveEnvelope('conversations.reset', reset('101')))
    expect(useStore.getState().homeConversationCache.summaryCursor ?? null).toBeNull()
    // The new store reuses id `a` and `dead` at low revisions; `gone` is not in it.
    emit(liveEnvelope('conversations.snapshot', summarySnapshot(
        [summary('a', '/p', 1, 'NEW'), summary('dead', '/p', 1, 'Reborn')],
        { watermark: '1', generation: 'generation-new' },
    )))
    expect(listed('/p')).toEqual(expect.arrayContaining([['a', 'NEW', '1'], ['dead', 'Reborn', '1']]))
    expect(listed('/p')).toHaveLength(2)
    expect(listed('/q')).toEqual([])
    expect(useStore.getState().homeConversationCache.summaryCursor).toEqual({ generation: 'generation-new', watermark: '1' })
    // An open transcript is not part of the summary scope and stays.
    expect(useStore.getState().homeConversationCache.conversationsById.open).toBeDefined()
})

describe('a project thread list that was in flight when the feed initialized', () => {
    /** Starts a project list request and returns its resolver. */
    const pendingList = async () => {
        const { renderHook } = await import('@testing-library/react')
        const { useProjectConversationCache } = await import('@/features/projects/hooks/useProjectConversationCache')
        let answer!: (body: unknown) => void
        fetchMock.mockImplementationOnce(() => new Promise((resolve) => {
            answer = (body) => resolve({ ok: true, status: 200, json: async () => body })
        }))
        const cache = renderHook(() => useProjectConversationCache({
            persistProjectState: async () => {}, projectSessionsByPath: {}, updateProjectSessionState: () => {},
        }))
        let request!: Promise<unknown>
        act(() => { request = cache.result.current.loadProjectConversationSummaries('/p') })
        return async (body: unknown) => { await act(async () => { answer(body); await request }) }
    }

    it('is ignored after the feed moved to a new store generation, and live changes go on', async () => {
        seedConversation('open', '/q', 2)
        render(<WorkspaceLiveEventsController />)
        emit(liveEnvelope('conversations.snapshot', summarySnapshot([summary('old', '/p', 100)], { watermark: '100', generation: 'generation-old' })))
        const respond = await pendingList()
        emit(liveEnvelope('conversations.reset', reset('100')))
        emit(liveEnvelope('conversations.snapshot', summarySnapshot(
            [summary('fresh', '/p', 1), summary('elsewhere', '/q', 1)],
            { watermark: '1', generation: 'generation-new' },
        )))
        // The old generation's list lands late.
        await respond(summarySnapshot([summary('old', '/p', 100)], { project_path: '/p', watermark: '100', generation: 'generation-old' }))
        expect(listed('/p')).toEqual([['fresh', 'Chat fresh', '1']])
        expect(listed('/q')).toEqual([['elsewhere', 'Chat elsewhere', '1']])
        expect(useStore.getState().homeConversationCache.summaryCursor).toEqual({ generation: 'generation-new', watermark: '1' })
        emit(changesEnvelope([conversationRow(summary('later', '/p', 2))], { watermark: '2', generation: 'generation-new' }))
        expect(listed('/p')).toEqual(expect.arrayContaining([['fresh', 'Chat fresh', '1'], ['later', 'Chat later', '2']]))
        expect(useStore.getState().homeConversationCache.conversationsById.open).toBeDefined()
    })

    it('read before a same-generation snapshot cannot bring back a thread it no longer lists', async () => {
        seedConversation('open', '/q', 2)
        render(<WorkspaceLiveEventsController />)
        const respond = await pendingList()
        emit(snapshotEnvelope([summary('kept', '/q', 150)], '200'))
        await respond(summarySnapshot([summary('gone', '/p', 90)], { project_path: '/p', watermark: '100' }))
        expect(listed('/p')).toEqual([])
        expect(listed('/q')).toEqual([['kept', 'Chat kept', '150']])
        expect(useStore.getState().homeConversationCache.summaryCursor).toEqual({ generation: TEST_GENERATION, watermark: '200' })
        emit(changesEnvelope([conversationRow(summary('new', '/p', 201))], { watermark: '201' }))
        expect(listed('/p')).toEqual([['new', 'Chat new', '201']])
        expect(useStore.getState().homeConversationCache.conversationsById.open).toBeDefined()
    })
})
