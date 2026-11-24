import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { compareRevisions } from '@/lib/api/conversationsApi'

import type { ProjectConversationSummary } from './types'

/** What a chat row's dot says: it waits on you, it is working, it changed since you last looked, or neither. */
export type ChatStatus = 'needs-you' | 'working' | 'unread' | 'read'

// The newest revision of each chat you have had open, and when this browser started tracking, per
// environment: a chat with no entry counts as read unless it changed after that.
// ponytail: per browser, and entries for deleted chats stay; move to the server if read state must follow you.
type ReadState = { since: number; revisions: Record<string, string> }

const STORAGE_KEY = 'spark.chat_read_revisions'
const listeners = new Set<() => void>()
// Parsed once per stored value, so a snapshot stays the same object until storage changes; keyed
// by storage key, as environments can hold chats with the same ids.
const cache = new Map<string, { raw: string | null; state: ReadState }>()
const storageKey = () => `${STORAGE_KEY}:${getActiveEnvironmentId() ?? 'local'}`

function readRaw(key: string): string | null {
    try {
        return window.localStorage.getItem(key)
    } catch {
        return cache.get(key)?.raw ?? null
    }
}

export function readChatReadState(): ReadState {
    const key = storageKey()
    const raw = readRaw(key)
    const cached = cache.get(key)
    if (cached && cached.raw === raw) return cached.state
    let state: ReadState | null = null
    try {
        const parsed = JSON.parse(raw ?? 'null') as Partial<ReadState> | null
        if (parsed && typeof parsed.since === 'number' && parsed.revisions && typeof parsed.revisions === 'object') {
            state = { since: parsed.since, revisions: parsed.revisions }
        }
    } catch {
        // Unreadable storage starts fresh below.
    }
    if (!state) return write(key, { since: Date.now(), revisions: {} })
    cache.set(key, { raw, state })
    return state
}

function write(key: string, state: ReadState): ReadState {
    let raw: string | null = JSON.stringify(state)
    try {
        window.localStorage.setItem(key, raw)
    } catch {
        // Storage refuses the write (private mode, quota, etc.): read state lasts the session, cached
        // against what storage still holds, so the snapshot stays the same until storage changes.
        raw = readRaw(key)
    }
    cache.set(key, { raw, state })
    return state
}

/** Records that you have seen a chat at this revision; never moves back. */
export function markChatRead(conversationId: string, revision: string) {
    const state = readChatReadState()
    const seen = state.revisions[conversationId]
    if (seen !== undefined && compareRevisions(revision, seen) <= 0) return
    write(storageKey(), { since: state.since, revisions: { ...state.revisions, [conversationId]: revision } })
    listeners.forEach((listener) => listener())
}

export function subscribeChatReadState(listener: () => void) {
    listeners.add(listener)
    return () => { listeners.delete(listener) }
}

export function chatStatus(summary: ProjectConversationSummary, state: ReadState): ChatStatus {
    if (summary.needs_attention) return 'needs-you'
    if (summary.running) return 'working'
    const seen = state.revisions[summary.conversation_id]
    const unread = seen === undefined
        ? Date.parse(summary.updated_at) > state.since
        : compareRevisions(summary.revision, seen) > 0
    return unread ? 'unread' : 'read'
}
