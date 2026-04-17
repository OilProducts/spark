import {
    parseConversationSnapshotResponse,
} from '@/lib/api/conversationsApi'

describe('conversationsApi parsing', () => {
    it('parses chat_mode and mode_change turns from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 4,
            conversation_id: 'conversation-plan',
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project-plan',
            chat_mode: 'plan',
            title: 'Planning thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            turns: [
                {
                    id: 'turn-mode-1',
                    role: 'system',
                    kind: 'mode_change',
                    status: 'complete',
                    content: 'plan',
                    timestamp: '2026-04-16T18:00:01Z',
                },
            ],
            segments: [],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.chat_mode).toBe('plan')
        expect(snapshot.turns[0]).toMatchObject({
            kind: 'mode_change',
            role: 'system',
            content: 'plan',
        })
    })
})
