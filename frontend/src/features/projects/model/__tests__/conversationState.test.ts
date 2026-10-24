import { describe, expect, it } from 'vitest'

import { parseConversationSummaryResponse } from '@/lib/api/conversationsApi'

import { sortConversationSummaries } from '../conversationState'

const summary = (id: string, updatedAt: string, standingRole?: unknown) => parseConversationSummaryResponse({
    conversation_id: id,
    conversation_handle: '',
    project_path: '/project',
    title: id,
    created_at: updatedAt,
    updated_at: updatedAt,
    revision: 1,
    launched_run_ids: [],
    standing_role: standingRole,
})!

describe('conversation summaries', () => {
    it('parses a standing role and leaves ordinary chats without one', () => {
        expect(summary('coord', '', { name: 'coordinator', title: 'Coordinator', state: 'working' }).standing_role)
            .toEqual({ name: 'coordinator', title: 'Coordinator', state: 'working' })
        expect(summary('odd', '', { name: 'cos', title: 'CoS', state: 'unknown' }).standing_role?.state).toBe('idle')
        expect(summary('chat', '2026-09-01T00:00:00Z').standing_role).toBeNull()
    })

    it('sorts standing roles first, the Coordinator leading, then newest first', () => {
        const sorted = sortConversationSummaries([
            summary('old', '2026-09-01T00:00:00Z'),
            summary('cos', '2026-08-01T00:00:00Z', { name: 'chief-of-staff', title: 'Chief of Staff', state: 'idle' }),
            summary('new', '2026-09-02T00:00:00Z'),
            summary('coord', '', { name: 'coordinator', title: 'Coordinator', state: 'idle' }),
        ])
        expect(sorted.map((entry) => entry.conversation_id)).toEqual(['coord', 'cos', 'new', 'old'])
    })
})
