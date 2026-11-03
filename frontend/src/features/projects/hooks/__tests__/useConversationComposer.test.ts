import type { SetStateAction } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { parseConversationComposerCommand, useConversationComposer } from '@/features/projects/hooks/useConversationComposer'
import type { PendingConversationTurnState } from '@/features/projects/model/conversationState'
import { sendConversationTurnValidated, type ConversationSnapshotResponse } from '@/lib/workspaceClient'
import { useStore } from '@/store'

vi.mock('@/lib/workspaceClient', async (original) => ({ ...await original<object>(), sendConversationTurnValidated: vi.fn() }))

describe('parseConversationComposerCommand', () => {
    it('parses bare mode switches', () => {
        expect(parseConversationComposerCommand('/plan')).toEqual({
            kind: 'switch_mode',
            chatMode: 'plan',
        })
        expect(parseConversationComposerCommand('/chat')).toEqual({
            kind: 'switch_mode',
            chatMode: 'chat',
        })
    })

    it('parses switch-and-send commands', () => {
        expect(parseConversationComposerCommand('/plan Draft the implementation plan.')).toEqual({
            kind: 'switch_and_send',
            chatMode: 'plan',
            message: 'Draft the implementation plan.',
        })
        expect(parseConversationComposerCommand('/chat Send the concise answer.')).toEqual({
            kind: 'switch_and_send',
            chatMode: 'chat',
            message: 'Send the concise answer.',
        })
    })

    it('leaves unknown slash commands alone', () => {
        expect(parseConversationComposerCommand('/unknown something')).toBeNull()
        expect(parseConversationComposerCommand('/planner')).toBeNull()
    })
})

describe('onChatComposerSubmit', () => {
    const projectPath = '/tmp/project'
    let pending: PendingConversationTurnState | null = null
    const setPending = vi.fn((value: SetStateAction<PendingConversationTurnState | null>) => {
        pending = typeof value === 'function' ? value(pending) : value
    })

    const composer = (overrides: Partial<Parameters<typeof useConversationComposer>[0]> = {}) => useConversationComposer({
        activeProjectPath: projectPath,
        chatDraft: 'Hello there',
        attachments: [],
        isUploadBlocking: false,
        isChatInputDisabled: false,
        ensureConversationId: () => 'conversation-1',
        getCurrentConversationId: () => 'conversation-1',
        getCurrentConversationRevision: () => 3,
        applyConversationSnapshot: vi.fn(),
        formatErrorMessage: (_error, fallback) => fallback,
        setChatDraft: vi.fn(),
        setPanelError: vi.fn(),
        setPendingConversationTurn: setPending,
        ...overrides,
    })
    const draft = () => useStore.getState().homeProjectSessionsByPath[projectPath]?.chatDraft
    const pendingMessages = () => useStore.getState().homeConversationSessionsById['conversation-1']?.pendingMessages ?? []

    beforeEach(() => {
        useStore.setState(useStore.getInitialState(), true)
        useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Hello there' })
        pending = null
        setPending.mockClear()
        vi.mocked(sendConversationTurnValidated).mockReset()
    })

    it('shows the message as pending under the id it sends, before the server answers', async () => {
        let respond!: (snapshot: ConversationSnapshotResponse) => void
        vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise((resolve) => { respond = resolve }))
        const applyConversationSnapshot = vi.fn()
        composer({ applyConversationSnapshot }).onChatComposerSubmit()
        await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

        const request = vi.mocked(sendConversationTurnValidated).mock.calls[0][1]
        expect(request.client_turn_id).toMatch(/^turn-[0-9a-f]{32}$/)
        expect(pendingMessages()).toHaveLength(1)
        expect(pendingMessages()[0]).toMatchObject({
            id: request.client_turn_id,
            role: 'user',
            content: 'Hello there',
            status: 'pending',
        })
        expect(draft()).toBe('')

        respond({ revision: 4 } as ConversationSnapshotResponse)
        await vi.waitFor(() => expect(applyConversationSnapshot).toHaveBeenCalled())
        expect(pending).toBeNull()
        expect(draft()).toBe('')
        // It stays until the server's record of it arrives, which may follow the response.
        expect(pendingMessages().map((message) => message.id)).toEqual([request.client_turn_id])
    })

    it('keeps a failed message and the draft typed while it was sending', async () => {
        let reject!: (error: Error) => void
        vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise((_, fail) => { reject = fail }))
        const setPanelError = vi.fn()
        composer({ setPanelError }).onChatComposerSubmit()
        await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())
        expect(draft()).toBe('')

        useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'A newer draft' })
        reject(new Error('offline'))

        await vi.waitFor(() => expect(setPanelError).toHaveBeenCalledWith('Unable to send the project chat turn.'))
        expect(pendingMessages()).toEqual([])
        expect(draft()).toBe('Hello there\n\nA newer draft')
    })

    it('reports a failed send and puts the message text back', async () => {
        vi.mocked(sendConversationTurnValidated).mockRejectedValue(new Error('offline'))
        const setPanelError = vi.fn()
        composer({ setPanelError }).onChatComposerSubmit()

        await vi.waitFor(() => expect(setPanelError).toHaveBeenCalledWith('Unable to send the project chat turn.'))
        expect(pending).toBeNull()
        expect(pendingMessages()).toEqual([])
        expect(draft()).toBe('Hello there')
    })
})
