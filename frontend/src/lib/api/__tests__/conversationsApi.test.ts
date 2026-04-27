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
            model: 'gpt-5.4',
            reasoning_effort: 'high',
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
        expect(snapshot.model).toBe('gpt-5.4')
        expect(snapshot.reasoning_effort).toBe('high')
        expect(snapshot.turns[0]).toMatchObject({
            kind: 'mode_change',
            role: 'system',
            content: 'plan',
        })
    })

    it('parses assistant turn token usage from snapshots', () => {
        const tokenUsage = {
            last: {
                inputTokens: 120,
                cachedInputTokens: 20,
                outputTokens: 18,
                reasoningOutputTokens: 5,
                totalTokens: 138,
            },
            total: {
                inputTokens: 200,
                cachedInputTokens: 30,
                outputTokens: 44,
                reasoningOutputTokens: 12,
                totalTokens: 244,
            },
        }
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 4,
            conversation_id: 'conversation-usage',
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project-plan',
            chat_mode: 'chat',
            title: 'Usage thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            turns: [
                {
                    id: 'turn-assistant-1',
                    role: 'assistant',
                    kind: 'message',
                    status: 'complete',
                    content: 'Done.',
                    timestamp: '2026-04-16T18:00:01Z',
                    token_usage: tokenUsage,
                },
            ],
            segments: [],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.turns[0].token_usage).toEqual(tokenUsage)
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

    it('preserves continuity-reset workflow event fields from snapshots', () => {
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
            segments: [],
            event_log: [
                {
                    message: 'Persisted thread thread-stale could not be resumed.',
                    timestamp: '2026-04-16T18:00:08Z',
                    kind: 'continuity_reset',
                    error_code: 'continuity_reset',
                    details: {
                        persisted_thread_id: 'thread-stale',
                        replacement_thread_started: false,
                        resume_failure: {
                            kind: 'resume_failed',
                            code: -32001,
                            message: 'Persisted thread missing from runtime',
                        },
                    },
                },
            ],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.event_log).toEqual([
            {
                message: 'Persisted thread thread-stale could not be resumed.',
                timestamp: '2026-04-16T18:00:08Z',
                kind: 'continuity_reset',
                error_code: 'continuity_reset',
                details: {
                    persisted_thread_id: 'thread-stale',
                    replacement_thread_started: false,
                    resume_failure: {
                        kind: 'resume_failed',
                        code: -32001,
                        message: 'Persisted thread missing from runtime',
                    },
                },
            },
        ])
    })

    it('parses proposed plan artifacts from snapshots', () => {
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
            segments: [],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
            proposed_plans: [
                {
                    id: 'proposed-plan-1',
                    created_at: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:09Z',
                    title: 'Reviewable proposed plans',
                    content: '# Reviewable proposed plans',
                    project_path: '/tmp/project-plan',
                    conversation_id: 'conversation-plan',
                    source_turn_id: 'turn-assistant-1',
                    source_segment_id: 'segment-plan-1',
                    status: 'pending_review',
                },
            ],
        })

        expect(snapshot.proposed_plans).toEqual([
            expect.objectContaining({
                id: 'proposed-plan-1',
                status: 'pending_review',
                source_segment_id: 'segment-plan-1',
            }),
        ])
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

    it('parses request_user_input segments from snapshots', () => {
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
                    id: 'segment-request-user-input-1',
                    turn_id: 'turn-assistant-1',
                    order: 1,
                    kind: 'request_user_input',
                    role: 'system',
                    status: 'pending',
                    timestamp: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:08Z',
                    content: 'Which path should I take?',
                    request_user_input: {
                        request_id: 'request-1',
                        status: 'pending',
                        questions: [
                            {
                                id: 'path_choice',
                                header: 'Path',
                                question: 'Which path should I take?',
                                question_type: 'MULTIPLE_CHOICE',
                                options: [
                                    {
                                        label: 'Inline card',
                                        description: 'Keep the request inside the timeline.',
                                    },
                                ],
                                allow_other: true,
                                is_secret: false,
                            },
                        ],
                        answers: {},
                        submitted_at: null,
                    },
                    source: {
                        app_turn_id: 'app-turn-1',
                        item_id: 'request-1',
                    },
                },
            ],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.segments[0]).toMatchObject({
            kind: 'request_user_input',
            role: 'system',
            request_user_input: {
                request_id: 'request-1',
                status: 'pending',
                questions: [
                    {
                        id: 'path_choice',
                        question: 'Which path should I take?',
                    },
                ],
            },
        })
    })

    it('parses expired request_user_input status from snapshots', () => {
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
                    id: 'segment-request-user-input-1',
                    turn_id: 'turn-assistant-1',
                    order: 1,
                    kind: 'request_user_input',
                    role: 'system',
                    status: 'failed',
                    timestamp: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:08Z',
                    content: 'Which path should I take?\nAnswer: Inline card',
                    request_user_input: {
                        request_id: 'request-1',
                        status: 'expired',
                        questions: [],
                        answers: {
                            path_choice: 'Inline card',
                        },
                        submitted_at: '2026-04-16T18:00:09Z',
                    },
                    source: {
                        app_turn_id: 'app-turn-1',
                        item_id: 'request-1',
                    },
                },
            ],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.segments[0]).toMatchObject({
            kind: 'request_user_input',
            request_user_input: {
                request_id: 'request-1',
                status: 'expired',
                answers: {
                    path_choice: 'Inline card',
                },
            },
        })
    })
})
