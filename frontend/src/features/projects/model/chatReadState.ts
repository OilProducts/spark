import { compareRevisions } from '@/lib/api/conversationsApi'

import type { ProjectConversationSummary } from './types'

/** What a chat row's dot says: it waits on you, it is working, it changed since you last looked, or neither. */
export type ChatStatus = 'needs-you' | 'working' | 'unread' | 'read'

// The newest revision of each chat you have had open, and when this browser started tracking:
// a chat with no entry counts as read unless it changed after that.
// ponytail: per browser, and entries for deleted chats stay; move to the server if read state must follow you.
type ReadState = { since: number; revisions: Record<string, string> }

const STORAGE_KEY = 'spark.chat_read_revisions'
const listeners = new Set<() => void>()
// Parsed once per stored value, so a snapshot stays the same object until storage changes.
let cache: { raw: string | null; state: ReadState } | null = null

function readRaw(): string | null {
    try {
        return window.localStorage.getItem(STORAGE_KEY)
    } catch {
        return cache?.raw ?? null
    }
}

export function readChatReadState(): ReadState {
    const raw = readRaw()
    if (cache && cache.raw === raw) return cache.state
    let state: ReadState | null = null
    try {
        const parsed = JSON.parse(raw ?? 'null') as Partial<ReadState> | null
        if (parsed && typeof parsed.since === 'number' && parsed.revisions && typeof parsed.revisions === 'object') {
            state = { since: parsed.since, revisions: parsed.revisions }
        }
    } catch {
        // Unreadable storage starts fresh below.
    }
    if (!state) return write({ since: Date.now(), revisions: {} })
    cache = { raw, state }
    return state
}

function write(state: ReadState): ReadState {
    const raw = JSON.stringify(state)
    try {
        window.localStorage.setItem(STORAGE_KEY, raw)
    } catch {
        // Ignore storage failures (private mode, quota, etc.); read state then lasts the session.
    }
    cache = { raw, state }
    return state
}

/** Records that you have seen a chat at this revision; never moves back. */
export function markChatRead(conversationId: string, revision: string) {
    const state = readChatReadState()
    const seen = state.revisions[conversationId]
    if (seen !== undefined && compareRevisions(revision, seen) <= 0) return
    write({ since: state.since, revisions: { ...state.revisions, [conversationId]: revision } })
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
