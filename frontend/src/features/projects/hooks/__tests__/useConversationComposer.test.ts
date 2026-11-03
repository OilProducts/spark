import type { SetStateAction } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { parseConversationComposerCommand, useConversationComposer } from '@/features/projects/hooks/useConversationComposer'
import type { PendingConversationTurnState } from '@/features/projects/model/conversationState'
import { applyConversationBatchToCache, applyConversationPageToCache } from '@/features/projects/model/projectsHomeState'
import {
    parseConversationAcceptanceResponse,
    parseConversationMutationAckResponse,
    parseConversationPageResponse,
    sendConversationTurnValidated,
    updateConversationSettingsValidated,
    type ConversationAcceptance,
    type ConversationBatch,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { conversationAcceptance, conversationBatch, conversationMutationAck, conversationPage, conversationRow, turnRow } from '@/test/conversationWire'

vi.mock('@/lib/workspaceClient', async (original) => ({
    ...await original<object>(),
    sendConversationTurnValidated: vi.fn(),
    updateConversationSettingsValidated: vi.fn(),
}))

const thread = (revision: number, overrides: Record<string, unknown> = {}) => ({
    conversation_id: 'conversation-1',
    project_path: '/tmp/project',
    title: 'Thread',
    created_at: '2026-09-01T00:00:00Z',
    updated_at: '2026-09-01T00:00:00Z',
    revision,
    turns: [],
    segments: [],
    ...overrides,
})
const record = () => useStore.getState().homeConversationCache.conversationsById['conversation-1']
// The shared reducer behind the composer, as the controller wires it.
const applyBatchToStore = (_projectPath: string, conversationId: string, batch: ConversationBatch, kind: 'live' | 'mutation') => {
    useStore.getState().commitHomeConversationCache((cache) => applyConversationBatchToCache(cache, conversationId, batch, kind))
}

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
        getConversationRecord: () => record(),
        applyConversationBatch: vi.fn(),
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
        vi.mocked(updateConversationSettingsValidated).mockReset()
    })

    it('shows the message as pending under the id it sends, before the server answers', async () => {
        let respond!: (acceptance: ConversationAcceptance) => void
        vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise((resolve) => { respond = resolve }))
        const applyConversationBatch = vi.fn()
        composer({ applyConversationBatch }).onChatComposerSubmit()
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

        const acceptance = parseConversationAcceptanceResponse(conversationAcceptance(thread(4), request.client_turn_id!))
        respond(acceptance)
        await vi.waitFor(() => expect(applyConversationBatch).toHaveBeenCalled())
        // Acceptance rows merge as the request's own result; they never move the live cursor.
        expect(applyConversationBatch).toHaveBeenCalledWith(projectPath, 'conversation-1', acceptance.changes, 'mutation', 'send-response', { forceWorkspaceSync: true })
        expect(pending).toBeNull()
        expect(draft()).toBe('')
        // It stays until the server's record of it arrives, which may follow the response.
        expect(pendingMessages().map((message) => message.id)).toEqual([request.client_turn_id])
    })

    it('sends from a non-secure origin, where crypto.randomUUID is missing', async () => {
        vi.stubGlobal('crypto', { getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto) })
        try {
            vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise(() => {}))
            composer().onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            const request = vi.mocked(sendConversationTurnValidated).mock.calls[0][1]
            expect(request.client_turn_id).toMatch(/^turn-[0-9a-f]{32}$/)
            expect(pendingMessages()).toMatchObject([{ id: request.client_turn_id, content: 'Hello there', status: 'pending' }])
        } finally {
            vi.unstubAllGlobals()
        }
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

    it('merges the accepted user turn under the id it sent, without moving the live cursor', async () => {
        useStore.getState().commitHomeConversationCache((cache) => applyConversationPageToCache(cache, parseConversationPageResponse(conversationPage(thread(3))), 'window'))
        vi.mocked(sendConversationTurnValidated).mockImplementation(async (_id, payload) => {
            const sent = { id: payload.client_turn_id, role: 'user', kind: 'message', status: 'complete', content: payload.message, timestamp: '2026-09-01T00:01:00Z' }
            return parseConversationAcceptanceResponse(conversationAcceptance(thread(5), payload.client_turn_id!, {
                rows: [conversationRow(thread(5)), turnRow('conversation-1', sent, '5')],
                status: 'queued',
            }))
        })
        composer({ applyConversationBatch: applyBatchToStore }).onChatComposerSubmit()
        await vi.waitFor(() => expect(record()?.orderedTurnIds).toHaveLength(1))

        const clientTurnId = vi.mocked(sendConversationTurnValidated).mock.calls[0][1].client_turn_id
        expect(record()?.orderedTurnIds).toEqual([clientTurnId])
        // The pending entry and the recorded turn share the id, so the transcript shows it once.
        expect(pendingMessages().map((message) => message.id)).toEqual([clientTurnId])
        expect(record()?.watermark).toBe('3')
    })
})

describe('switchChatMode', () => {
    const projectPath = '/tmp/project'
    const composer = () => useConversationComposer({
        activeProjectPath: projectPath,
        chatDraft: '',
        attachments: [],
        isUploadBlocking: false,
        isChatInputDisabled: false,
        ensureConversationId: () => 'conversation-1',
        getCurrentConversationId: () => 'conversation-1',
        getConversationRecord: () => record(),
        applyConversationBatch: applyBatchToStore,
        formatErrorMessage: (_error, fallback) => fallback,
        setChatDraft: vi.fn(),
        setPanelError: vi.fn(),
        setPendingConversationTurn: vi.fn(),
    })

    beforeEach(() => {
        useStore.setState(useStore.getInitialState(), true)
        vi.mocked(updateConversationSettingsValidated).mockReset()
        useStore.getState().commitHomeConversationCache((cache) => applyConversationPageToCache(cache, parseConversationPageResponse(conversationPage(thread(3))), 'window'))
    })

    it('expects the conversation row revision and takes the mode from the acknowledged row', async () => {
        vi.mocked(updateConversationSettingsValidated).mockResolvedValue(parseConversationMutationAckResponse(conversationMutationAck(thread(6, { chat_mode: 'plan' }), [
            conversationRow(thread(6, { chat_mode: 'plan' })),
            turnRow('conversation-1', { id: 'mode-1', role: 'system', kind: 'mode_change', status: 'complete', content: 'plan', timestamp: '2026-09-01T00:02:00Z' }, '6'),
        ])))
        await composer().onSwitchChatMode('plan')

        expect(vi.mocked(updateConversationSettingsValidated).mock.calls[0][1]).toMatchObject({ chat_mode: 'plan', expected_revision: '3' })
        expect(record()).toMatchObject({ chat_mode: 'plan', revision: '6', watermark: '3' })
        expect(record()?.orderedTurnIds).toEqual(['mode-1'])
    })

    it('expects the settings revision, which trails a row bumped by other changes', async () => {
        // An upload re-delivers the row at 9 without changing settings committed at 3.
        applyBatchToStore(projectPath, 'conversation-1', parseConversationMutationAckResponse(conversationMutationAck(thread(9, { settings_revision: 3 }))).changes, 'mutation')
        expect(record()).toMatchObject({ revision: '9', settingsRevision: '3' })
        vi.mocked(updateConversationSettingsValidated).mockResolvedValue(parseConversationMutationAckResponse(conversationMutationAck(thread(10, { chat_mode: 'plan' }))))
        await composer().onSwitchChatMode('plan')

        expect(vi.mocked(updateConversationSettingsValidated).mock.calls[0][1]).toMatchObject({ expected_revision: '3' })
        expect(record()).toMatchObject({ chat_mode: 'plan', settingsRevision: '10' })
    })

    it('applies an unchanged-settings acknowledgement as its current row', async () => {
        vi.mocked(updateConversationSettingsValidated).mockResolvedValue(parseConversationMutationAckResponse({
            generation: 'generation-test',
            revision: '3',
            changes: conversationBatch([conversationRow(thread(3))], { from_revision: '9', watermark: '9' }),
        }))
        const before = record()
        await composer().onSwitchChatMode('chat')

        expect(record()).toBe(before)
        expect(record()).toMatchObject({ chat_mode: 'chat', revision: '3', watermark: '3' })
    })

    it('keeps a live update that overtook the settings response', async () => {
        vi.mocked(updateConversationSettingsValidated).mockImplementation(async () => {
            // The stream delivers a newer conversation row before the response lands.
            applyBatchToStore(projectPath, 'conversation-1', {
                generation: 'generation-test', from_revision: '3', watermark: '8',
                rows: parseConversationMutationAckResponse(conversationMutationAck(thread(8, { chat_mode: 'chat', title: 'Live' }))).changes.rows,
            }, 'live')
            return parseConversationMutationAckResponse(conversationMutationAck(thread(7, { chat_mode: 'plan' })))
        })
        await composer().onSwitchChatMode('plan')

        expect(record()).toMatchObject({ chat_mode: 'chat', title: 'Live', revision: '8', watermark: '8' })
    })
})
