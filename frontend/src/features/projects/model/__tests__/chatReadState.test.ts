import { describe, expect, it } from 'vitest'

import { chatStatus, markChatRead, readChatReadState } from '../chatReadState'
import type { ProjectConversationSummary } from '../types'

const chat = (overrides: Partial<ProjectConversationSummary> = {}): ProjectConversationSummary => ({
    conversation_id: 'c1',
    project_path: '/tmp/p',
    title: 'Chat',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    revision: '5',
    ...overrides,
})

describe('chatStatus', () => {
    it('puts waiting on you ahead of working, and working ahead of read state', () => {
        const state = readChatReadState()
        expect(chatStatus(chat({ needs_attention: true, running: true }), state)).toBe('needs-you')
        expect(chatStatus(chat({ running: true }), state)).toBe('working')
    })

    it('counts a chat never opened as read unless it changed after tracking began', () => {
        const state = readChatReadState()
        expect(chatStatus(chat(), state)).toBe('read')
        expect(chatStatus(chat({ updated_at: new Date(state.since + 1000).toISOString() }), state)).toBe('unread')
    })

    it('is unread once the chat moves past the revision you saw, and read again when you see it', () => {
        markChatRead('c1', '5')
        expect(chatStatus(chat(), readChatReadState())).toBe('read')
        expect(chatStatus(chat({ revision: '12' }), readChatReadState())).toBe('unread')
        markChatRead('c1', '12')
        // An older revision arriving late does not move it back.
        markChatRead('c1', '9')
        expect(readChatReadState().revisions.c1).toBe('12')
        expect(chatStatus(chat({ revision: '12' }), readChatReadState())).toBe('read')
    })
})
