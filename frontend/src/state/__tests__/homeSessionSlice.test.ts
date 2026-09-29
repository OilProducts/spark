import { beforeEach, describe, expect, it } from 'vitest'

import { useStore } from '@/store'
import type { ConversationSummaryResponse } from '@/lib/api/conversationsApi'

const summary = (id: string, title: string, revision: number, project_path = '/tmp/project'): ConversationSummaryResponse => ({
    conversation_id: id,
    conversation_handle: null,
    project_path,
    title,
    created_at: '2026-09-28T10:00:00Z',
    updated_at: '2026-09-28T10:00:00Z',
    revision,
    last_message_preview: null,
})

describe('upsertHomeConversationSummary', () => {
    beforeEach(() => {
        useStore.setState({ homeConversationCache: { conversationsById: {}, summariesByProjectPath: {} } })
        useStore.getState().setHomeConversationSummaryList('/tmp/project', [
            summary('a', 'first message snippet', 2),
            summary('b', 'other thread', 5),
        ])
    })

    const titles = () => useStore.getState().homeConversationCache.summariesByProjectPath['/tmp/project']
        .map((entry) => [entry.conversation_id, entry.title])

    it('updates a loaded thread list, including threads not being viewed', () => {
        useStore.getState().upsertHomeConversationSummary(summary('a', 'Generated title', 3))
        expect(titles()).toContainEqual(['a', 'Generated title'])
        expect(titles()).toContainEqual(['b', 'other thread'])
    })

    it('ignores older revisions and projects whose list is not loaded', () => {
        useStore.getState().upsertHomeConversationSummary(summary('b', 'stale', 4))
        useStore.getState().upsertHomeConversationSummary(summary('c', 'elsewhere', 1, '/tmp/other'))
        expect(titles()).toContainEqual(['b', 'other thread'])
        expect(useStore.getState().homeConversationCache.summariesByProjectPath['/tmp/other']).toBeUndefined()
    })
})
