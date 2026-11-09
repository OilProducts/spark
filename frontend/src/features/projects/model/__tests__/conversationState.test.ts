import { describe, expect, it } from 'vitest'

import { conversationSummaryFromRow, parseConversationBatchResponse, parseConversationSummarySnapshotResponse, type ConversationRow } from '@/lib/api/conversationsApi'
import { artifactRow, conversationBatch, summarySnapshot } from '@/test/conversationWire'

import { conversationRowKey, mergeConversationRows, sortConversationSummaries } from '../conversationState'

const summary = (id: string, updatedAt: string, standingRole?: unknown, revision = 1) => {
    const row = parseConversationSummarySnapshotResponse(summarySnapshot([{
        conversation_id: id,
        conversation_handle: '',
        project_path: '/project',
        title: id,
        created_at: updatedAt,
        updated_at: updatedAt,
        revision,
        launched_run_ids: [],
        standing_role: standingRole ?? null,
    }])).rows[0] as Extract<ConversationRow, { kind: 'conversation' }>
    return conversationSummaryFromRow({ ...row, payload: row.payload! })
}

describe('conversation summaries', () => {
    it('parses a standing role and leaves ordinary chats without one', () => {
        expect(summary('coord', '', { name: 'coordinator', title: 'Coordinator', state: 'working' }).standing_role)
            .toEqual({ name: 'coordinator', title: 'Coordinator', state: 'working' })
        expect(summary('odd', '', { name: 'cos', title: 'CoS', state: 'unknown' }).standing_role?.state).toBe('idle')
        expect(summary('chat', '2026-09-01T00:00:00Z').standing_role).toBeNull()
    })

    it('sorts standing roles first by name, then newest first', () => {
        const sorted = sortConversationSummaries([
            summary('old', '2026-09-01T00:00:00Z'),
            summary('coord', '2026-09-03T00:00:00Z', { name: 'coordinator', title: 'Coordinator', state: 'idle' }),
            summary('new', '2026-09-02T00:00:00Z'),
            summary('arch', '2026-08-01T00:00:00Z', { name: 'archivist', title: 'Archivist', state: 'idle' }),
        ])
        expect(sorted.map((entry) => entry.conversation_id)).toEqual(['arch', 'coord', 'new', 'old'])
    })

    it('breaks an updated-at tie by revision, compared as integers', () => {
        const sorted = sortConversationSummaries([
            summary('lower', '2026-09-01T00:00:00Z', undefined, 9),
            summary('higher', '2026-09-01T00:00:00Z', undefined, 10),
        ])
        expect(sorted.map((entry) => entry.conversation_id)).toEqual(['higher', 'lower'])
    })
})

describe('row identity and merge', () => {
    const artifact = (collection: string, id: string, revision: string) => parseConversationBatchResponse(conversationBatch([
        artifactRow('c', collection, collection === 'event_log'
            ? { message: id, timestamp: '2026-09-01T00:00:00Z' }
            : { id, created_at: '', updated_at: '', flow_name: 'f', summary: id, project_path: '/p', conversation_id: 'c', source_turn_id: 't', title: id, content: id }, revision, 1),
    ])).rows[0]

    it('keys artifacts by collection, so equal ids in two collections are two rows', () => {
        const merged = mergeConversationRows({}, [artifact('flow_run_requests', 'a1', '1'), artifact('flow_launches', 'a1', '1')])
        expect(Object.keys(merged)).toHaveLength(2)
        expect(conversationRowKey({ kind: 'turn', conversation_id: 'c', id: 'a1' })).not.toBe(conversationRowKey({ kind: 'segment', conversation_id: 'c', id: 'a1' }))
    })

    it('replaces a row only with a higher revision and returns the same object otherwise', () => {
        const rows = mergeConversationRows({}, [artifact('flow_launches', 'a1', '5')])
        expect(mergeConversationRows(rows, [artifact('flow_launches', 'a1', '5')])).toBe(rows)
        expect(mergeConversationRows(rows, [artifact('flow_launches', 'a1', '4')])).toBe(rows)
        const next = mergeConversationRows(rows, [artifact('flow_launches', 'a1', '18446744073709551617')])
        expect(Object.values(next)[0].revision).toBe('18446744073709551617')
    })
})
