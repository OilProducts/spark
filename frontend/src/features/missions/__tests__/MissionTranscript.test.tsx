import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, it, vi } from 'vitest'
import type { ConversationSnapshotResponse } from '@/lib/workspaceClient'
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

it.each(['reasoning', 'plan'] as const)('offers sign-in and Retry on a failed mission turn with only a %s segment', async (kind) => {
    retry.mockClear()
    const reload = vi.fn()
    render(<MissionTranscript conversation={{ conversationId: 'mission-1', snapshot: snapshot(kind), loading: false, error: '', reload }} project="/project" runs={new Map()} />)

    expect(screen.getByText('Claude Code needs sign-in. Your conversation is saved.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sign in to Claude' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }))
    expect(retry).toHaveBeenCalledWith('mission-1', 'turn-failed', '/project')
    await waitFor(() => expect(reload).toHaveBeenCalled())
})
