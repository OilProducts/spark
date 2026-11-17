import { render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'

import { useStore } from '@/store'
import { markChatRead } from '../../model/chatReadState'
import type { ProjectConversationSummary } from '../../model/types'
import { ProjectsSidebar } from '../ProjectsSidebar'

const PROJECT = '/tmp/status-project'
const later = new Date(Date.now() + 60_000).toISOString()
const chat = (id: string, overrides: Partial<ProjectConversationSummary> = {}): ProjectConversationSummary => ({
    conversation_id: id,
    conversation_handle: `${id}-handle`,
    project_path: PROJECT,
    title: id,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    revision: '1',
    ...overrides,
})

function renderSidebar(chats: ProjectConversationSummary[], activeConversationId: string | null) {
    return render(
        <ProjectsSidebar
            activeProjectPath={PROJECT}
            activeConversationId={activeConversationId}
            conversationSummariesByProjectPath={{ [PROJECT]: chats }}
            conversationSummariesStatusByProjectPath={{ [PROJECT]: 'ready' }}
            pendingDeleteConversationId={null}
            onCreateConversationThread={() => {}}
            onSelectConversationThread={() => {}}
            onDeleteConversationThread={() => {}}
            formatConversationAgeShort={() => '2h'}
        />,
    )
}

const row = (id: string) => document.querySelector<HTMLElement>(`[data-testid="chats-chat-row"][data-conversation-id="${id}"]`)!

it('marks each chat as waiting on you, working, unread or read, and reads the open one', () => {
    useStore.getState().registerProject(PROJECT)
    useStore.getState().setViewMode('home')
    markChatRead('read', '1')
    markChatRead('unread', '1')
    const chats = [
        chat('waiting', { needs_attention: true }),
        chat('working', { running: true }),
        chat('unread', { revision: '4' }),
        chat('read'),
        chat('fresh', { updated_at: later }),
    ]
    const { rerender } = renderSidebar(chats, null)

    expect(row('waiting')).toHaveAttribute('data-chat-status', 'needs-you')
    expect(screen.getByText('Needs you')).toBeInTheDocument()
    expect(row('working')).toHaveAttribute('data-chat-status', 'working')
    expect(screen.getByText('Working…')).toBeInTheDocument()
    expect(row('unread')).toHaveAttribute('data-chat-status', 'unread')
    expect(row('unread')).toHaveAccessibleDescription(/^Unread ·\s*unread-handle · 2h$/)
    expect(row('read')).toHaveAttribute('data-chat-status', 'read')
    // A chat that changed after tracking began, never opened here, is new to you.
    expect(row('fresh')).toHaveAttribute('data-chat-status', 'unread')

    // Opening a chat reads it.
    rerender(
        <ProjectsSidebar
            activeProjectPath={PROJECT}
            activeConversationId="unread"
            conversationSummariesByProjectPath={{ [PROJECT]: chats }}
            conversationSummariesStatusByProjectPath={{ [PROJECT]: 'ready' }}
            pendingDeleteConversationId={null}
            onCreateConversationThread={() => {}}
            onSelectConversationThread={() => {}}
            onDeleteConversationThread={() => {}}
            formatConversationAgeShort={() => '2h'}
        />,
    )
    expect(row('unread')).toHaveAttribute('data-chat-status', 'read')
})

it('leaves the open chat unread while another view is showing', () => {
    useStore.getState().registerProject(PROJECT)
    useStore.getState().setViewMode('runs')
    markChatRead('open', '1')
    renderSidebar([chat('open', { revision: '3' })], 'open')
    expect(row('open')).toHaveAttribute('data-chat-status', 'unread')
})
