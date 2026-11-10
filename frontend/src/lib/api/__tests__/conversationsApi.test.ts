import {
    ConversationResetError,
    conversationSummaryFromRow,
    fetchConversationChangesValidated,
    fetchConversationPageValidated,
    fetchProjectConversationListValidated,
    parseConversationAcceptanceResponse,
    parseConversationBatchResponse,
    parseConversationMutationAckResponse,
    parseConversationPageResponse,
    parseConversationResetResponse,
    parseConversationSummarySnapshotResponse,
    type ConversationRow,
} from '@/lib/api/conversationsApi'
import { afterEach, vi } from 'vitest'
import { artifactRow, conversationBatch, conversationPage, conversationRow, segmentRow, summarySnapshot, tombstoneRow, turnRow } from '@/test/conversationWire'

// The page a transcript-shaped fixture becomes, with its rows' payloads grouped by kind.
function parseConversationSnapshotResponse(fixture: Record<string, unknown>) {
    const events = Array.isArray(fixture.event_log) ? fixture.event_log as Record<string, unknown>[] : []
    const raw = conversationPage(fixture, { revision: '1' })
    const page = parseConversationPageResponse({
        ...raw,
        rows: [...raw.rows as unknown[], ...events.map((event, index) => artifactRow(String(fixture.conversation_id), 'event_log', { ...event, id: `event-${index}` }, '1', index + 1))],
    })
    const payloads = <K extends ConversationRow['kind']>(kind: K) => page.rows
        .filter((row): row is Extract<ConversationRow, { kind: K }> => row.kind === kind)
        .map((row) => row.payload!)
    const artifacts = (collection: string) => page.rows.flatMap((row) => row.kind === 'artifact' && row.collection === collection ? [row.payload] : [])
    const conversation = payloads('conversation')[0]!
    return {
        ...conversation,
        turns: payloads('turn'),
        segments: payloads('segment'),
        event_log: artifacts('event_log').map((event) => {
            const { id: _id, ...rest } = event as Record<string, unknown>
            return rest
        }),
        proposed_plans: artifacts('proposed_plans'),
    }
}

const batchOf = (rows: Record<string, unknown>[]) => parseConversationBatchResponse(conversationBatch(rows))

describe('conversationsApi parsing', () => {
    it('parses chat_mode and mode_change turns from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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

    it('parses truncated tool output metadata from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 5,
            revision: 0,
            conversation_id: 'conversation-tool-output',
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project-plan',
            chat_mode: 'chat',
            title: 'Tool output thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            turns: [],
            segments: [
                {
                    id: 'segment-tool-1',
                    turn_id: 'turn-assistant-1',
                    order: 1,
                    kind: 'tool_call',
                    role: 'system',
                    status: 'complete',
                    timestamp: '2026-04-16T18:00:08Z',
                    updated_at: '2026-04-16T18:00:08Z',
                    content: '',
                    tool_call: {
                        id: 'tool-call-1',
                        kind: 'command_execution',
                        status: 'completed',
                        title: 'List files',
                        output: 'preview',
                        output_size: 12000,
                        output_truncated: true,
                        file_paths: [],
                    },
                },
            ],
            event_log: [],
            flow_run_requests: [],
            flow_launches: [],
        })

        expect(snapshot.segments[0].tool_call).toMatchObject({
            output: 'preview',
            output_size: 12000,
            output_truncated: true,
        })
    })

    it('preserves continuity-reset workflow event fields from snapshots', () => {
        const snapshot = parseConversationSnapshotResponse({
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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
            schema_version: 5,
            revision: 0,
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

    it('rejects a batch whose turn row has a numeric revision', () => {
        const row = turnRow('conversation-plan', {
            id: 'turn-assistant-1',
            role: 'assistant',
            kind: 'message',
            status: 'streaming',
            content: '',
            timestamp: '2026-04-16T18:00:01Z',
        }, '1')
        expect(() => parseConversationBatchResponse({ generation: 'g', from_revision: '0', watermark: '1', rows: [{ ...row, revision: 1 }] })).toThrow()
    })

    it('rejects a batch whose segment row lacks its order or names another turn', () => {
        const segment = {
            id: 'segment-assistant-1',
            turn_id: 'turn-assistant-1',
            order: 1,
            kind: 'assistant_message',
            role: 'assistant',
            status: 'streaming',
            timestamp: '2026-04-16T18:00:08Z',
            updated_at: '2026-04-16T18:00:08Z',
            content: 'Hello',
        }
        const row = segmentRow('conversation-plan', segment, '1')
        expect(batchOf([row]).rows).toHaveLength(1)
        expect(() => batchOf([{ ...row, order: undefined }])).toThrow()
        expect(() => batchOf([{ ...row, turn_id: 'turn-other' }])).toThrow()
    })

    it('parses artifact rows that arrive with their segment', () => {
        const conversationId = 'conversation-plan'
        const base = { created_at: '2026-04-16T18:00:08Z', updated_at: '2026-04-16T18:00:08Z', project_path: '/tmp/project-plan', conversation_id: conversationId, source_turn_id: 'turn-assistant-1' }
        const batch = batchOf([
            segmentRow(conversationId, {
                id: 'segment-plan-1',
                turn_id: 'turn-assistant-1',
                order: 1,
                kind: 'plan',
                role: 'assistant',
                status: 'complete',
                timestamp: '2026-04-16T18:00:08Z',
                updated_at: '2026-04-16T18:00:08Z',
                content: 'Plan content.',
                artifact_id: 'plan-1',
            }, '1'),
            artifactRow(conversationId, 'proposed_plans', { ...base, id: 'plan-1', title: 'Plan', content: 'Plan content.', source_segment_id: 'segment-plan-1', status: 'pending_review' }, '1', 1),
            artifactRow(conversationId, 'flow_run_requests', { ...base, id: 'request-1', flow_name: 'implementation.dot', summary: 'Run implementation.', source_segment_id: 'segment-request-1', status: 'pending' }, '1', 1),
            artifactRow(conversationId, 'flow_launches', { ...base, id: 'launch-1', flow_name: 'implementation.dot', summary: 'Launch implementation.', source_segment_id: 'segment-launch-1', status: 'pending' }, '1', 1),
        ])

        const artifacts = batch.rows.flatMap((row) => row.kind === 'artifact' ? [[row.collection, (row.payload as { id: string }).id]] : [])
        expect(artifacts).toEqual([['proposed_plans', 'plan-1'], ['flow_run_requests', 'request-1'], ['flow_launches', 'launch-1']])
        expect(() => batchOf([artifactRow(conversationId, 'unknown_collection', { id: 'x' }, '1', 1)])).toThrow()
        expect(() => batchOf([artifactRow(conversationId, 'proposed_plans', { id: 'plan-2' }, '1', 1)])).toThrow()
    })

    it('parses a run recovery segment and its artifact', () => {
        const conversationId = 'conversation-users-chris-projects-spark-431aef86'
        const at = '2026-11-10T02:54:49Z'
        const recovery = {
            id: 'run-recovery-fafa28ed0684',
            operation: 'continue',
            status: 'started',
            source_run_id: 'run-18e67cc27559fe08',
            result_run_id: 'run-2026-11-10T02-54-49-275189000Z',
            start_node: 'verify_findings',
            flow_source_mode: 'snapshot',
            flow_name: null,
            model: null, llm_provider: null, llm_profile: null, reasoning_effort: null,
            project_path: '/Users/chris/projects/spark',
            conversation_id: conversationId,
            source_turn_id: 'turn-1fb2f664677e450284fafbc616388304',
            source_segment_id: 'segment-artifact-run-recovery-fafa28ed0684',
            created_at: at,
            updated_at: at,
        }
        const batch = batchOf([
            segmentRow(conversationId, {
                id: 'segment-artifact-run-recovery-fafa28ed0684',
                turn_id: 'turn-1fb2f664677e450284fafbc616388304',
                order: 7,
                kind: 'run_recovery',
                role: 'system',
                status: 'complete',
                timestamp: at,
                updated_at: at,
                content: '',
                artifact_id: 'run-recovery-fafa28ed0684',
                source: {},
            }, '3'),
            artifactRow(conversationId, 'run_recoveries', recovery, '3', 1),
        ])

        const [segment, artifact] = batch.rows
        expect(segment).toMatchObject({ kind: 'segment', payload: { kind: 'run_recovery', artifact_id: 'run-recovery-fafa28ed0684' } })
        expect(artifact).toMatchObject({ kind: 'artifact', collection: 'run_recoveries', payload: {
            id: 'run-recovery-fafa28ed0684',
            operation: 'continue',
            status: 'started',
            source_run_id: 'run-18e67cc27559fe08',
            result_run_id: 'run-2026-11-10T02-54-49-275189000Z',
            start_node: 'verify_findings',
        } })

        // A continue that failed before starting a run records an empty result id and its error.
        const failed = batchOf([artifactRow(conversationId, 'run_recoveries', { ...recovery, status: 'failed', result_run_id: '', recovery_error: 'Unknown node.' }, '4', 1)])
        expect(failed.rows[0]).toMatchObject({ payload: { status: 'failed', result_run_id: null, recovery_error: 'Unknown node.' } })
        expect(() => batchOf([artifactRow(conversationId, 'run_recoveries', { ...recovery, operation: 'rewind' }, '4', 1)])).toThrow()
    })

    it('parses yielded tool calls with their completion reason', () => {
        const batch = batchOf([segmentRow('conversation-yield', {
            id: 'segment-tool-app-1-exec-a',
            turn_id: 'turn-1',
            order: 2,
            kind: 'tool_call',
            role: 'system',
            status: 'complete',
            timestamp: '2026-08-27T11:59:58Z',
            updated_at: '2026-08-27T12:00:00Z',
            content: '',
            tool_call: {
                id: 'exec-a',
                kind: 'command_execution',
                status: 'yielded',
                completion_reason: 'turn_boundary_yield',
                title: 'Run command',
                command: 'sleep 60',
                output: 'partial',
            },
        }, '7')])
        const row = batch.rows[0]
        expect(row.kind).toBe('segment')
        if (row.kind !== 'segment') {
            return
        }
        expect(row.payload?.tool_call?.status).toBe('yielded')
        expect(row.payload?.tool_call?.completion_reason).toBe('turn_boundary_yield')
    })

    it('parses tombstone rows without a payload', () => {
        const batch = batchOf([tombstoneRow('segment', 'conversation-yield', 'segment-agent-event-turn-1-turn_completed-6', '9', { turn_id: 'turn-1' })])
        expect(batch.rows).toEqual([{
            kind: 'segment',
            conversation_id: 'conversation-yield',
            id: 'segment-agent-event-turn-1-turn_completed-6',
            turn_id: 'turn-1',
            revision: '9',
            deleted: true,
            payload: null,
        }])
    })
})

describe('conversation wire envelopes', () => {
    const conversationId = 'conversation-1'
    const fixture = {
        conversation_id: conversationId,
        project_path: '/tmp/project',
        title: 'Thread',
        created_at: '2026-04-16T18:00:00Z',
        updated_at: '2026-04-16T18:00:10Z',
        revision: 4,
        chat_mode: 'plan',
        provider: 'codex',
        model: 'gpt-5.4',
        turns: [{ id: 'turn-1', role: 'user', kind: 'message', status: 'complete', content: 'Hi.', timestamp: '2026-04-16T18:00:01Z' }],
        segments: [],
        settings: { models: { scope: 'conversation', source: 'conversation', revision: '4', stored: null, effective: { provider: 'codex', llm_profile: null, model: 'gpt-5.4', reasoning_effort: null } } },
    }

    it('parses a page with its conversation row, window and cursor', () => {
        const page = parseConversationPageResponse(conversationPage(fixture, { older_cursor: 'cursor-1' }))
        expect(page).toMatchObject({ generation: 'generation-test', watermark: '4', conversation_id: conversationId, turn_ids: ['turn-1'], older_cursor: 'cursor-1' })
        const conversation = page.rows.find((row) => row.kind === 'conversation')
        expect(conversation?.payload).toMatchObject({ chat_mode: 'plan', model: 'gpt-5.4', title: 'Thread' })
        expect(conversation?.kind === 'conversation' && conversation.payload?.model_settings_view?.revision).toBe('4')
    })

    it('rejects a page row above its watermark or from another conversation', () => {
        expect(() => parseConversationPageResponse(conversationPage(fixture, { watermark: '3' }))).toThrow()
        const raw = conversationPage(fixture)
        expect(() => parseConversationPageResponse({ ...raw, rows: [...raw.rows as unknown[], conversationRow({ ...fixture, conversation_id: 'other' })] })).toThrow()
    })

    it('parses an acceptance and fills client_turn_id from user_turn_id', () => {
        const changes = conversationBatch([turnRow(conversationId, fixture.turns[0], '5')], { from_revision: '0' })
        expect(parseConversationAcceptanceResponse({ generation: 'g', revision: '5', user_turn_id: 'turn-1', assistant_turn_id: null, status: 'queued', changes }))
            .toMatchObject({ user_turn_id: 'turn-1', client_turn_id: 'turn-1', assistant_turn_id: null, status: 'queued', revision: '5' })
        expect(() => parseConversationAcceptanceResponse({ generation: 'g', revision: '5', user_turn_id: 'turn-1', status: 'sent', changes })).toThrow()
    })

    it('parses a mutation acknowledgement whose unchanged row is at or below its start', () => {
        const ack = parseConversationMutationAckResponse({ generation: 'g', revision: '4', changes: conversationBatch([conversationRow(fixture, '4')], { from_revision: '9', watermark: '9' }) })
        expect(ack.changes.rows).toHaveLength(1)
        expect(() => parseConversationMutationAckResponse({ generation: 'g', revision: '4', changes: conversationBatch([conversationRow(fixture, '10')], { from_revision: '9', watermark: '9' }) })).toThrow()
    })

    it('parses resets', () => {
        expect(parseConversationResetResponse({ code: 'window_reset', reason: 'order_changed', generation: 'g', watermark: '12' }))
            .toEqual({ code: 'window_reset', reason: 'order_changed', generation: 'g', watermark: '12' })
        expect(() => parseConversationResetResponse({ code: 'window_reset', reason: 'because', generation: 'g', watermark: '12' })).toThrow()
    })

    it('parses a summary snapshot and maps its rows to thread-list entries', () => {
        const role = { name: 'coordinator', title: 'Coordinator', state: 'working' }
        const snapshot = parseConversationSummarySnapshotResponse(summarySnapshot([
            { ...fixture, conversation_handle: 'steady-harbor', last_message_preview: 'Hi.', launched_run_ids: ['run-1'], running: true, standing_role: role },
        ], { project_path: null }))
        expect(snapshot).toMatchObject({ project_path: null, watermark: '4' })
        const row = snapshot.rows[0]
        if (row.kind !== 'conversation' || !row.payload) throw new Error('expected a conversation row')
        expect(conversationSummaryFromRow({ ...row, payload: row.payload })).toEqual({
            conversation_id: conversationId,
            conversation_handle: 'steady-harbor',
            project_path: '/tmp/project',
            title: 'Thread',
            created_at: '2026-04-16T18:00:00Z',
            updated_at: '2026-04-16T18:00:10Z',
            revision: '4',
            last_message_preview: 'Hi.',
            launched_run_ids: ['run-1'],
            standing_role: role,
            running: true,
        })
        expect(() => parseConversationSummarySnapshotResponse({ generation: 'g', watermark: '4', project_path: null, rows: [turnRow(conversationId, fixture.turns[0], '1')] })).toThrow()
    })
})

describe('conversation reads', () => {
    afterEach(() => {
        vi.unstubAllGlobals()
    })

    const respond = (status: number, body: unknown) => {
        const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }))
        vi.stubGlobal('fetch', fetchMock)
        return fetchMock
    }

    it('requests the newest page, then the page before a cursor', async () => {
        const fetchMock = respond(200, conversationPage({ conversation_id: 'c 1', project_path: '/p', title: 'T', revision: 2, turns: [], segments: [] }))
        await fetchConversationPageValidated('c 1', '/p')
        await fetchConversationPageValidated('c 1', '/p', 'cur/sor')
        expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
            '/workspace/api/conversations/c%201?project_path=%2Fp&limit=50',
            '/workspace/api/conversations/c%201?project_path=%2Fp&limit=50&before=cur%2Fsor',
        ])
    })

    it('surfaces a 409 reset as a typed error', async () => {
        respond(409, { code: 'window_reset', reason: 'order_changed', generation: 'g', watermark: '7' })
        const error = await fetchConversationPageValidated('c', '/p', 'cursor').catch((caught: unknown) => caught)
        expect(error).toBeInstanceOf(ConversationResetError)
        expect((error as ConversationResetError).reset).toMatchObject({ code: 'window_reset', reason: 'order_changed' })

        respond(409, { code: 'resync_required', reason: 'catchup_limit', generation: 'g', watermark: '7' })
        await expect(fetchConversationChangesValidated('c', '/p', 'g', '3')).rejects.toBeInstanceOf(ConversationResetError)
    })

    it('keeps other 409s as plain HTTP errors', async () => {
        respond(409, { detail: 'turn_id_conflict: reused' })
        const error = await fetchConversationPageValidated('c', '/p').catch((caught: unknown) => caught)
        expect(error).not.toBeInstanceOf(ConversationResetError)
        expect(error).toMatchObject({ status: 409, detail: 'turn_id_conflict: reused' })
    })

    it('requests changes since a revision and lists summaries for one or every project', async () => {
        const fetchMock = respond(200, { generation: 'g', from_revision: '3', watermark: '3', rows: [] })
        await expect(fetchConversationChangesValidated('c', '/p', 'g', '3')).resolves.toMatchObject({ watermark: '3', rows: [] })
        respond(200, summarySnapshot([]))
        const listMock = vi.mocked(fetch)
        await fetchProjectConversationListValidated('/p')
        await fetchProjectConversationListValidated(null)
        expect(fetchMock.mock.calls[0][0]).toBe('/workspace/api/conversations/c/changes?project_path=%2Fp&generation=g&since=3')
        expect(listMock.mock.calls.map(([url]) => url)).toEqual([
            '/workspace/api/projects/conversations?project_path=%2Fp',
            '/workspace/api/projects/conversations',
        ])
    })
})
