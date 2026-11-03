import { beforeEach, describe, expect, it } from 'vitest'

import { useStore } from '@/store'
import { applySummaryBatchToCache, applySummarySnapshotToCache } from '@/features/projects/model/projectsHomeState'
import { parseConversationBatchResponse, parseConversationSummarySnapshotResponse } from '@/lib/workspaceClient'
import { conversationBatch, conversationRow, summarySnapshot } from '@/test/conversationWire'

const summary = (id: string, title: string, revision: number, project_path = '/tmp/project') => ({
    conversation_id: id,
    conversation_handle: null,
    project_path,
    title,
    created_at: '2026-09-28T10:00:00Z',
    updated_at: '2026-09-28T10:00:00Z',
    revision,
    last_message_preview: null,
})

// A live summary change for one conversation, as `conversations.changes` carries it.
const upsertSummary = (entry: ReturnType<typeof summary>) => useStore.getState().commitHomeConversationCache((cache) => (
    applySummaryBatchToCache(cache, parseConversationBatchResponse(conversationBatch([conversationRow(entry)])))
))

describe('summary changes in the home conversation cache', () => {
    beforeEach(() => {
        useStore.setState({ homeConversationCache: { conversationsById: {}, summariesByProjectPath: {} } })
        useStore.getState().commitHomeConversationCache((cache) => applySummarySnapshotToCache(
            cache,
            parseConversationSummarySnapshotResponse(summarySnapshot([
                summary('a', 'first message snippet', 2),
                summary('b', 'other thread', 5),
            ], { project_path: '/tmp/project' })),
            'http',
        ))
    })

    const titles = () => useStore.getState().homeConversationCache.summariesByProjectPath['/tmp/project']
        .map((entry) => [entry.conversation_id, entry.title])

    it('updates a loaded thread list, including threads not being viewed', () => {
        upsertSummary(summary('a', 'Generated title', 3))
        expect(titles()).toContainEqual(['a', 'Generated title'])
        expect(titles()).toContainEqual(['b', 'other thread'])
    })

    it('ignores older revisions', () => {
        upsertSummary(summary('b', 'stale', 4))
        expect(titles()).toContainEqual(['b', 'other thread'])
    })

    it('starts a list for a project whose list is not loaded yet', () => {
        upsertSummary(summary('c', 'elsewhere', 1, '/tmp/other'))
        const list = useStore.getState().homeConversationCache.summariesByProjectPath['/tmp/other']
        expect(list).toHaveLength(1)
        expect(list[0]).toMatchObject({ ...summary('c', 'elsewhere', 1, '/tmp/other'), revision: '1', last_message_preview: undefined })
    })
})
