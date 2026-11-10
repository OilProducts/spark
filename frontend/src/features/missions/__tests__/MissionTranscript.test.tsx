import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, it, vi } from 'vitest'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'
import { parseConversationPageResponse } from '@/lib/workspaceClient'
import { applyConversationPage } from '@/features/projects/model/projectsHomeState'
import { conversationPage } from '@/test/conversationWire'
import { buildRunsHash } from '@/app/runsRouting'
import { MissionTranscript } from '../MissionTranscript'

const retry = vi.hoisted(() => vi.fn(async () => ({})))
vi.mock('@/lib/api/conversationsApi', async (importActual) => ({
    ...await importActual<typeof import('@/lib/api/conversationsApi')>(),
    retryConversationTurnValidated: retry,
}))

const at = '2026-03-10T10:00:00Z'
const snapshot = (segmentKind: 'reasoning' | 'plan'): ConversationSnapshotResponse => ({
    schema_version: 5, revision: 0, conversation_id: 'mission-1', conversation_handle: 'mission-1', project_path: '/project',
    chat_mode: 'chat', title: 'Mission', created_at: at, updated_at: at,
    turns: [
        { id: 'turn-user', role: 'user', content: 'Deliver the prompt.', timestamp: at, kind: 'message', status: 'complete', artifact_id: null },
        { id: 'turn-failed', role: 'assistant', content: '', timestamp: at, kind: 'message', status: 'failed', artifact_id: null, error: 'Failed to authenticate: 401' },
    ],
    segments: [{
        id: 'only-segment', turn_id: 'turn-failed', order: 1, kind: segmentKind, role: 'assistant', status: 'failed', timestamp: at,
        updated_at: at, completed_at: at, content: 'Partial work.', artifact_id: null, error: null, source: null, tool_call: null,
    }],
    event_log: [], flow_run_requests: [], flow_launches: [], proposed_plans: [],
})
// Turns share a timestamp, so each carries its transcript position.
const positioned = (fixture: ConversationSnapshotResponse) => ({ ...fixture, turns: fixture.turns.map((turn, index) => ({ ...turn, position: index + 1 })) })
const record = (fixture: ConversationSnapshotResponse) => applyConversationPage(undefined, parseConversationPageResponse(conversationPage(positioned(fixture))), 'window') ?? null

it.each(['reasoning', 'plan'] as const)('offers sign-in and Retry on a failed mission turn with only a %s segment', async (kind) => {
    retry.mockClear()
    const reload = vi.fn()
    render(<MissionTranscript conversation={{ conversationId: 'mission-1', record: record(snapshot(kind)), loading: false, error: '', reload }} project="/project" runs={new Map()} />)

    expect(screen.getByText('Claude Code needs sign-in. Your conversation is saved.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sign in to Claude' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }))
    expect(retry).toHaveBeenCalledWith('mission-1', 'turn-failed', '/project')
    await waitFor(() => expect(reload).toHaveBeenCalled())
})

it('shows a message delivered into the running turn where it was sent, and one queued behind it', () => {
    const segment = (id: string, order: number, kind: 'assistant_message' | 'user_message', content: string) => ({
        id, turn_id: 'turn-running', order, kind, role: kind === 'user_message' ? 'user' as const : 'assistant' as const, status: 'complete' as const,
        timestamp: at, updated_at: at, completed_at: at, content, artifact_id: null, error: null, source: null, tool_call: null,
    })
    const live: ConversationSnapshotResponse = {
        ...snapshot('reasoning'),
        turns: [
            { id: 'turn-user', role: 'user', content: 'Deliver the prompt.', timestamp: at, kind: 'message', status: 'complete', artifact_id: null },
            { id: 'turn-running', role: 'assistant', content: '', timestamp: at, kind: 'message', status: 'streaming', artifact_id: null },
            { id: 'turn-queued', role: 'user', content: 'Then write the summary.', timestamp: at, kind: 'message', status: 'queued', artifact_id: null },
        ],
        segments: [
            segment('working', 1, 'assistant_message', 'Working.'),
            segment('delivered', 2, 'user_message', 'Use the docs.'),
            segment('reply', 3, 'assistant_message', 'Read your note.'),
        ],
    }
    render(<MissionTranscript conversation={{ conversationId: 'mission-1', record: record(live), loading: false, error: '', reload: vi.fn() }} project="/project" runs={new Map()} />)

    expect(screen.getByText('Sent into the running turn')).toBeInTheDocument()
    expect(screen.getByText('Queued · starts when the current turn ends')).toBeInTheDocument()
    const text = document.body.textContent ?? ''
    expect(text.indexOf('Working.')).toBeLessThan(text.indexOf('Use the docs.'))
    expect(text.indexOf('Use the docs.')).toBeLessThan(text.indexOf('Read your note.'))
    expect(text.indexOf('Read your note.')).toBeLessThan(text.indexOf('Then write the summary.'))
})

it('shows a message whose delivery into the running turn is unresolved as still sending, never as sent', () => {
    const live: ConversationSnapshotResponse = {
        ...snapshot('reasoning'),
        turns: [
            { id: 'turn-user', role: 'user', content: 'Deliver the prompt.', timestamp: at, kind: 'message', status: 'complete', artifact_id: null },
            { id: 'turn-running', role: 'assistant', content: '', timestamp: at, kind: 'message', status: 'streaming', artifact_id: null },
        ],
        segments: [{
            id: 'segment-user-message-sent-1', turn_id: 'turn-running', order: 1, kind: 'user_message', role: 'user',
            status: 'delivery_pending', timestamp: at, updated_at: at, completed_at: null, content: 'Use the docs.',
            artifact_id: null, error: null, source: null, tool_call: null,
        }],
    }
    render(<MissionTranscript conversation={{ conversationId: 'mission-1', record: record(live), loading: false, error: '', reload: vi.fn() }} project="/project" runs={new Map()} />)
    expect(screen.getByText('Sending into the running turn…')).toBeInTheDocument()
    expect(screen.queryByText('Sent into the running turn')).toBeNull()
})

it('shows run recoveries as run cards linking the resulting run, and a failed one with its error', () => {
    const segment = (id: string, order: number, kind: 'assistant_message' | 'run_recovery', extra: Record<string, unknown> = {}) => ({
        id, turn_id: 'turn-done', order, kind, role: kind === 'run_recovery' ? 'system' as const : 'assistant' as const, status: 'complete' as const,
        timestamp: at, updated_at: at, completed_at: at, content: '', artifact_id: null, error: null, source: null, tool_call: null, ...extra,
    })
    const recovery = (id: string, overrides: Record<string, unknown>) => ({
        id, created_at: at, updated_at: at, status: 'started', source_run_id: 'run-source', result_run_id: 'run-result',
        project_path: '/project', conversation_id: 'mission-1', source_turn_id: 'turn-done', source_segment_id: `segment-artifact-${id}`, ...overrides,
    })
    const fixture = {
        ...snapshot('reasoning'),
        turns: [
            { id: 'turn-user', role: 'user', content: 'Deliver the prompt.', timestamp: at, kind: 'message', status: 'complete', artifact_id: null },
            { id: 'turn-done', role: 'assistant', content: '', timestamp: at, kind: 'message', status: 'complete', artifact_id: null },
        ],
        segments: [
            segment('answer', 1, 'assistant_message', { content: 'Done.' }),
            segment('segment-artifact-run-recovery-1', 2, 'run_recovery', { artifact_id: 'run-recovery-1' }),
            segment('segment-artifact-run-recovery-2', 3, 'run_recovery', { artifact_id: 'run-recovery-2' }),
            segment('segment-artifact-run-recovery-3', 4, 'run_recovery', { artifact_id: 'run-recovery-3' }),
        ],
        run_recoveries: [
            recovery('run-recovery-1', { operation: 'continue', start_node: 'verify_findings' }),
            recovery('run-recovery-2', { operation: 'continue', status: 'failed', result_run_id: '', start_node: 'missing', recovery_error: 'Unknown node.' }),
            recovery('run-recovery-3', { operation: 'retry', status: 'pending', result_run_id: '' }),
        ],
    } as ConversationSnapshotResponse
    render(<MissionTranscript conversation={{ conversationId: 'mission-1', record: record(fixture), loading: false, error: '', reload: vi.fn() }} project="/project" runs={new Map()} />)

    expect(screen.getByText('Done.')).toBeInTheDocument()
    const card = screen.getByTestId('mission-run-recovery-card')
    expect(card).toHaveTextContent('Run recovery · Continue run-source from verify_findings')
    expect(card).toHaveTextContent('Started')
    expect(screen.getByRole('link', { name: 'run-result' })).toHaveAttribute('href', buildRunsHash('run-result'))
    expect(screen.getByText(/failed: Unknown node\.$/).closest('div')).toHaveTextContent('Continue run-source from missing failed: Unknown node.')
    expect(screen.getByText('Retry run-source')).toBeInTheDocument()
    expect(screen.getAllByRole('link')).toHaveLength(1)
})
