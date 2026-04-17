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

    it('parses plan segments from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 4,
            conversation_id: 'conversation-plan',
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project-plan',
            chat_mode: 'plan',
            title: 'Planning thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            turns: [],
            segments: [
                {
                    id: 'segment-plan-1',
                    turn_id: 'turn-assistant-1',
                    order: 1,
                    kind: 'plan',
                    role: 'assistant',
                    status: 'complete',
                    timestamp: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:08Z',
                    completed_at: '2026-04-16T18:00:08Z',
                    content: '1. Add a regression test.',
                    source: {
                        app_turn_id: 'app-turn-1',
                        item_id: 'plan-1',
                    },
                },
            ],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.segments[0]).toMatchObject({
            kind: 'plan',
            content: '1. Add a regression test.',
        })
    })

    it('parses context_compaction segments from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 4,
            conversation_id: 'conversation-plan',
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project-plan',
            chat_mode: 'chat',
            title: 'Planning thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            turns: [],
            segments: [
                {
                    id: 'segment-context-compaction-app-turn-1',
                    turn_id: 'turn-assistant-1',
                    order: 1,
                    kind: 'context_compaction',
                    role: 'system',
                    status: 'complete',
                    timestamp: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:08Z',
                    completed_at: '2026-04-16T18:00:08Z',
                    content: 'Context compacted to continue the turn.',
                    source: {
                        app_turn_id: 'app-turn-1',
                    },
                },
            ],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.segments[0]).toMatchObject({
            kind: 'context_compaction',
            role: 'system',
            content: 'Context compacted to continue the turn.',
        })
    })
})
