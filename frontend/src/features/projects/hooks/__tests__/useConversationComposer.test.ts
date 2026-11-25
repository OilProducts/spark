import type { SetStateAction } from 'react'
import { renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { parseConversationComposerCommand, useConversationComposer } from '@/features/projects/hooks/useConversationComposer'
import { useComposerAttachments } from '@/features/projects/hooks/useComposerAttachments'
import type { PendingConversationTurnState } from '@/features/projects/model/conversationState'
import type { ComposerAttachment } from '@/features/projects/model/types'
import { applyConversationBatchToCache, applyConversationPageToCache, applySummarySnapshotToCache, toHydratedProjectRecord } from '@/features/projects/model/projectsHomeState'
import {
    deleteConversationAttachmentValidated,
    parseConversationAcceptanceResponse,
    parseConversationMutationAckResponse,
    parseConversationPageResponse,
    parseConversationSummarySnapshotResponse,
    sendConversationTurnValidated,
    updateConversationSettingsValidated,
    uploadConversationAttachmentValidated,
    type ConversationAcceptance,
    type ConversationBatch,
} from '@/lib/workspaceClient'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { deferred, switchEnvironment } from '@/test/environments'
import { conversationAcceptance, conversationBatch, conversationMutationAck, conversationPage, conversationRow, summarySnapshot, turnRow } from '@/test/conversationWire'

vi.mock('@/lib/workspaceClient', async (original) => ({
    ...await original<object>(),
    deleteConversationAttachmentValidated: vi.fn(async () => undefined),
    sendConversationTurnValidated: vi.fn(),
    updateConversationSettingsValidated: vi.fn(),
    uploadConversationAttachmentValidated: vi.fn(),
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
        expect(parseConversationComposerCommand('/models')).toBeNull()
        expect(parseConversationComposerCommand('/effortless')).toBeNull()
    })

    it('parses /model and /effort slash commands', () => {
        expect(parseConversationComposerCommand('/model')).toEqual({ kind: 'open_model_picker' })
        expect(parseConversationComposerCommand('/model   gpt-5.4  ')).toEqual({ kind: 'set_model', query: 'gpt-5.4' })
        expect(parseConversationComposerCommand('/model codex/gpt-5.4')).toEqual({ kind: 'set_model', query: 'codex/gpt-5.4' })
        expect(parseConversationComposerCommand('/effort high')).toEqual({ kind: 'set_effort', effort: 'high' })
        expect(parseConversationComposerCommand('/effort default')).toEqual({ kind: 'set_effort', effort: 'default' })
        expect(parseConversationComposerCommand('/effort')).toEqual({ kind: 'set_effort', effort: '' })
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
        // The project's chats show, open on the chat a refused send puts its message back in.
        useStore.getState().registerProject(projectPath)
        useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-1' })
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

    describe('across environment switches', () => {
        const show = async (environmentId: string | null) => {
            await switchEnvironment(environmentId)
            resetStoreForEnvironmentSwitch(environmentId === null)
        }
        const accept = (call: number) => parseConversationAcceptanceResponse(conversationAcceptance(
            thread(4 + call), vi.mocked(sendConversationTurnValidated).mock.calls[call][1].client_turn_id!,
        ))

        afterEach(() => show(null))

        const entry: ComposerAttachment = {
            localId: 'local-1', projectPath, file: new File(['image'], 'shot.png', { type: 'image/png' }), previewUrl: null,
            status: 'uploaded', error: null, attachment: { id: 'attachment-1', filename: 'shot.png', media_type: 'image/png', size_bytes: 5 },
        }
        const openChat = (path: string) => useStore.getState().projectSessionsByPath[path]?.conversationId ?? null
        // The projects load, as the workspace loads them, with the chat the server records as open.
        const loadProjects = (activeConversationId: string | null) => useStore.getState().hydrateProjectRegistry([
            toHydratedProjectRecord({ project_path: projectPath, is_favorite: false, active_conversation_id: activeConversationId }),
            toHydratedProjectRecord({ project_path: '/tmp/other', is_favorite: false }),
        ])
        const attachmentsOf = (conversationId: string) => useStore.getState().homeConversationSessionsById[conversationId]?.attachments
        // The project's chat list, as the Chats panel loads it.
        const listChats = (...conversationIds: string[]) => useStore.getState().commitHomeConversationCache((cache) => applySummarySnapshotToCache(cache,
            parseConversationSummarySnapshotResponse(summarySnapshot(conversationIds.map((conversation_id) => thread(3, { conversation_id })), { project_path: projectPath })),
            'http', projectPath))

        it('puts a refused attachment-only send back, attachments and error, once its environment shows its chats, even after a return before they load', async () => {
            await show('env-r')
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ chatDraft: '', attachments: [entry], setPanelError }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())
            expect(vi.mocked(sendConversationTurnValidated).mock.calls[0][1]).toMatchObject({ message: '', attachment_ids: ['attachment-1'] })

            await show(null)
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(useStore.getState().homeConversationSessionsById).toEqual({})
            expect(setPanelError.mock.calls).toEqual([[null]])

            // Back briefly, then away again before the projects load: nothing shows, nor is lost.
            await show('env-r')
            expect(setPanelError.mock.calls).toEqual([[null]])
            expect(attachmentsOf('conversation-1')).toBeUndefined()
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
            expect(attachmentsOf('conversation-1')).toEqual([entry])
        })

        describe('refused while another environment\'s connection is checked, its workspace closed', () => {
            // As the workspace sets it: in its environment's store, which a switch that goes through wipes.
            const setPanelError = (message: string | null) => useStore.getState().updateHomeProjectSession(projectPath, { panelError: message })
            const panelError = () => useStore.getState().homeProjectSessionsByPath[projectPath]?.panelError
            const refusedWhileChecking = async () => {
                loadProjects('conversation-1')
                useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [entry] })
                const sent = deferred<ConversationAcceptance>()
                vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
                composer({ chatDraft: '', attachments: [entry], setPanelError }).onChatComposerSubmit()
                await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())
                const checked = deferred<void>()
                const switching = switchEnvironment('env-b', checked.promise)
                sent.reject(new Error('offline'))
                await new Promise((resolve) => setTimeout(resolve, 0))
                return { checked, switching }
            }

            it('puts an attachment-only send back, attachment and error, on a return once the switch went through', async () => {
                const { checked, switching } = await refusedWhileChecking()
                checked.resolve()
                await switching
                resetStoreForEnvironmentSwitch(false)

                await show(null)
                loadProjects('conversation-1')
                expect(attachmentsOf('conversation-1')).toEqual([entry])
                expect(panelError()).toBe('Unable to send the project chat turn.')
            })

            it('puts it back once its workspace shows again after the connection failed', async () => {
                const { checked, switching } = await refusedWhileChecking()
                checked.reject(new Error('unreachable'))
                await expect(switching).rejects.toThrow()
                await switchEnvironment(null)

                // Its projects load again as its workspace opens.
                loadProjects('conversation-1')
                expect(attachmentsOf('conversation-1')).toEqual([entry])
                expect(panelError()).toBe('Unable to send the project chat turn.')
            })
        })

        it('keeps a send refused while another chat sends for its own chat, leaving that chat open with its draft and pending send', async () => {
            loadProjects('conversation-1')
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [entry] })
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            vi.mocked(uploadConversationAttachmentValidated).mockReset()
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Another chat opens and sends, and its next message is typed; the first send is refused meanwhile.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-2' })
            composer({ chatDraft: 'Second', ensureConversationId: () => 'conversation-2', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Third' })
            first.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))

            expect(setPanelError.mock.calls).toEqual([[null]])
            expect(openChat(projectPath)).toBe('conversation-2')
            expect(draft()).toBe('Third')
            expect(attachmentsOf('conversation-2')).toEqual([])
            expect(uploadConversationAttachmentValidated).not.toHaveBeenCalled()
            expect(pending).toMatchObject({ conversationId: 'conversation-2' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-2': true })
            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()

            // Its own chat, opened again, has its text, files and error back.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-1' })
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-1')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('leaves New chat fresh when a send from another chat is refused, keeping it for that chat', async () => {
            loadProjects('conversation-1')
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            // New chat opens, as its action does; the send is refused meanwhile.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: null })
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBeNull()
            expect(draft()).toBe('')
            expect(setPanelError.mock.calls).toEqual([[null]])

            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-1' })
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-1')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('opens a refused first send\'s unlisted chat again, with its text, files and error, once its environment shows its chats anew', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            // A new chat's first message, with a file, in the chat that sending opens; no list names it yet.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Hello there' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            // Another chat opens, then this machine shows; the send is refused meanwhile.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-1' })
            await show(null)
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(setPanelError.mock.calls).toEqual([[null]])

            // Back, its projects load on the chat opened since; the refused chat opens again with all of it.
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
            // The chat opened separately stays fresh.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-1' })
            expect(draft()).toBe('')
            expect(attachmentsOf('conversation-1')).toBeUndefined()
        })

        it('opens a new chat\'s refused first send again, here and now, when refused after a return whose projects loaded on another chat you have not changed', async () => {
            await show('env-r')
            loadProjects('conversation-1')
            listChats('conversation-1')
            // New chat's first message, files only, in the chat sending opens; saving it as the project's chat fails.
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ chatDraft: '', attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            // Away and back: the projects and Chats load on the chat the server still has open; then the send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            listChats('conversation-1')
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('leaves a chat started after a return as it is when an unlisted chat\'s send is refused late, opening that chat with all of it at the next return', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Hello there' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            // Away and back, its projects load, and New chat starts with its own draft; then the send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: null })
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Newer' })
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBeNull()
            expect(draft()).toBe('Newer')
            expect(setPanelError.mock.calls).toEqual([[null]])

            // The next return opens the refused chat with its text, files and error.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('leaves a chat its projects loaded on open, with its text-only send in flight, when an unlisted chat\'s send is refused late, opening that chat with all of it at the next return', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Away and back, its projects load on another chat, which sends a message, clearing the draft; then the first send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Second' })
            composer({ chatDraft: 'Second', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            expect(draft()).toBe('')
            first.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-1')
            expect(pending).toMatchObject({ conversationId: 'conversation-1' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
            expect(setPanelError.mock.calls).toEqual([[null]])
            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(openChat(projectPath)).toBe('conversation-1')

            // The next return opens the refused chat with its text, files and error.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('keeps a chat its projects loaded on open from the submit of its draft through its send when an unlisted chat\'s send was refused late, opening that chat with all of it at the next return', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Away and back, its projects load on another chat, where a message is typed; the first send is refused meanwhile.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Second' })
            first.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-1')

            // The typed message is sent, clearing the draft: its chat stays open throughout.
            const opened = new Set<string | null>()
            const stop = useStore.subscribe(() => opened.add(openChat(projectPath)))
            composer({ chatDraft: 'Second', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            expect(draft()).toBe('')
            expect(pending).toMatchObject({ conversationId: 'conversation-1' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            stop()
            expect([...opened]).toEqual(['conversation-1'])
            expect(setPanelError.mock.calls).toEqual([[null]])

            // The next return opens the refused chat with its text, files and error.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('keeps a chat its projects loaded on open while its message waits on a settings save and the composer is cleared, when an unlisted chat\'s send is refused late, opening that chat with all of it at the next return', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Away and back, its projects load on another chat, whose message waits on a model settings save; the
            // composer is cleared meanwhile, then the first send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Second' })
            const settings = deferred<boolean>()
            composer({ chatDraft: 'Second', getCurrentConversationId: openChat, waitForPendingSettingsSave: () => settings.promise }).onChatComposerSubmit()
            await new Promise((resolve) => setTimeout(resolve, 0))
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: '' })
            first.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-1')
            expect(setPanelError.mock.calls).toEqual([[null]])

            // The save lands: the message goes from its chat, which ends its own pending and sending state.
            settings.resolve(true)
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            expect(vi.mocked(sendConversationTurnValidated).mock.calls[1][0]).toBe('conversation-1')
            expect(pending).toMatchObject({ conversationId: 'conversation-1' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()
            expect(openChat(projectPath)).toBe('conversation-1')

            // The next return opens the refused chat with its text, files and error.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('opens a late-refused unlisted chat again, here and now, when refused only once the chat its projects loaded on has finished sending', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Away and back, its projects load on another chat, which sends a message that is accepted; then the first send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Second' })
            composer({ chatDraft: 'Second', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()
            first.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('opens a late-refused unlisted chat again, here and now, when the chat its projects loaded on only opens the model picker with /model', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            useStore.getState().updateProjectSessionState(projectPath, { conversationId: 'conversation-new' })
            useStore.getState().updateHomeConversationSession('conversation-new', { attachments: [entry] })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, ensureConversationId: () => 'conversation-new', getCurrentConversationId: openChat }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))

            // Away and back, its projects load on another chat, where /model is typed; the send is refused meanwhile.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: '/model' })
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(openChat(projectPath)).toBe('conversation-1')

            // It opens the picker and clears the draft, sending nothing: the refused chat opens with all of it.
            const onOpenModelPicker = vi.fn()
            const setChatDraft = (value: SetStateAction<string>) => useStore.getState().updateHomeProjectSession(projectPath, {
                chatDraft: typeof value === 'function' ? value(draft() ?? '') : value,
            })
            composer({ chatDraft: '/model', onOpenModelPicker, setChatDraft, getCurrentConversationId: openChat }).onChatComposerSubmit()
            expect(onOpenModelPicker).toHaveBeenCalledTimes(1)
            expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1)
            expect(openChat(projectPath)).toBe('conversation-new')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-new')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], ['Unable to send the project chat turn.']])
        })

        it('puts a send back unsent, with its files and why, when its settings fail to save after a switch away', async () => {
            loadProjects('conversation-1')
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [entry] })
            const settings = deferred<boolean | string>()
            const waitForPendingSettingsSave = vi.fn(() => settings.promise)
            const setPanelError = vi.fn()
            composer({ attachments: [entry], setPanelError, waitForPendingSettingsSave }).onChatComposerSubmit()
            await vi.waitFor(() => expect(waitForPendingSettingsSave).toHaveBeenCalled())

            await show('env-r')
            settings.resolve(false)
            await new Promise((resolve) => setTimeout(resolve, 0))
            await show(null)
            loadProjects('conversation-1')
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-1')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([['The message was not sent: its chat settings did not save.']])
            expect(sendConversationTurnValidated).not.toHaveBeenCalled()
        })

        const fileOf = (id: string): ComposerAttachment => ({ ...entry, localId: `local-${id}`, attachment: { ...entry.attachment!, id: `attachment-${id}` } })
        // The panel error as the workspace keeps it, in the store.
        const panelError = () => useStore.getState().homeProjectSessionsByPath[projectPath]?.panelError
        const setStoredPanelError = (message: string | null) => useStore.getState().updateHomeProjectSession(projectPath, { panelError: message })

        it('leaves a file removed while Send waits on its settings out of the send, and out of the draft once that is refused', async () => {
            loadProjects('conversation-1')
            const removed = fileOf('removed')
            const second = fileOf('2')
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [removed, second] })
            const settings = deferred<boolean | string>()
            vi.mocked(sendConversationTurnValidated).mockRejectedValue(new Error('offline'))
            const setPanelError = vi.fn()
            composer({ attachments: [removed, second], setPanelError, waitForPendingSettingsSave: () => settings.promise }).onChatComposerSubmit()

            // The first file is removed, and its upload deleted, while the settings save.
            const { result } = renderHook(() => useComposerAttachments({
                activeConversationId: 'conversation-1', activeProjectPath: projectPath, ensureConversationId: () => 'conversation-1', setPanelError,
            }))
            result.current.remove('local-removed')
            expect(deleteConversationAttachmentValidated).toHaveBeenCalledWith('conversation-1', 'attachment-removed', projectPath)
            settings.resolve(true)
            await vi.waitFor(() => expect(setPanelError).toHaveBeenLastCalledWith('Unable to send the project chat turn.'))
            expect(vi.mocked(sendConversationTurnValidated).mock.calls[0][1]).toMatchObject({ attachment_ids: ['attachment-2'] })
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-1')).toEqual([second])
        })

        it('keeps a late-refused send\'s files apart from a newer draft\'s until both fit one message, then puts them back with its text and error', async () => {
            await show('env-r')
            loadProjects('conversation-1')
            const refused = ['a', 'b', 'c', 'd', 'e', 'f'].map(fileOf)
            const newer = ['g', 'h', 'i', 'j', 'k', 'l'].map(fileOf)
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: refused })
            const sent = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(sent.promise)
            composer({ attachments: refused, setPanelError: setStoredPanelError }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalled())

            // Away and back, the chat opens again and a newer message is written with files of its own; then the send is refused.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: 'Newer' })
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: newer })
            sent.reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(draft()).toBe('Newer')
            expect(attachmentsOf('conversation-1')).toEqual(newer)
            expect(panelError()).toBe('Unable to send the project chat turn. It comes back once its files fit this chat\'s draft.')

            // The newer message goes, as an accepted send leaves the draft; the refused one comes back.
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: '' })
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [] })
            expect(draft()).toBe('Hello there')
            expect(attachmentsOf('conversation-1')).toEqual(refused)
            expect(panelError()).toBe('Unable to send the project chat turn.')
        })

        it('puts back one of two six-file sends refused for a chat while hidden on a return, keeping the other\'s text and files until both fit one message', async () => {
            const first = ['a', 'b', 'c', 'd', 'e', 'f'].map(fileOf)
            const second = ['g', 'h', 'i', 'j', 'k', 'l'].map(fileOf)
            const sends = [deferred<ConversationAcceptance>(), deferred<ConversationAcceptance>()]
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(sends[0].promise).mockReturnValueOnce(sends[1].promise)
            // Each goes from the chat with six files; a switch away and back between them starts its draft anew.
            const send = async (text: string, files: ComposerAttachment[], call: number) => {
                await show('env-r')
                loadProjects('conversation-1')
                useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: text })
                useStore.getState().updateHomeConversationSession('conversation-1', { attachments: files })
                composer({ chatDraft: text, attachments: files, setPanelError: setStoredPanelError }).onChatComposerSubmit()
                await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(call))
                await show(null)
            }
            await send('First', first, 1)
            await send('Second', second, 2)
            sends[0].reject(new Error('offline'))
            sends[1].reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))

            await show('env-r')
            loadProjects('conversation-1')
            expect(attachmentsOf('conversation-1')).toEqual(first)
            expect(draft()).toBe('First')
            expect(panelError()).toBe('Unable to send the project chat turn.')

            // Once the first goes, as an accepted send leaves the draft, the second comes back.
            useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: '' })
            useStore.getState().updateHomeConversationSession('conversation-1', { attachments: [] })
            expect(attachmentsOf('conversation-1')).toEqual(second)
            expect(draft()).toBe('Second')
            expect(panelError()).toBe('Unable to send the project chat turn.')
        })

        it('keeps two new chats\' refused first sends apart, opening one with its own text, file and error as its environment shows its chats again and the other at the next showing', async () => {
            await show('env-r')
            loadProjects(null)
            listChats('conversation-1')
            const sends = [deferred<ConversationAcceptance>(), deferred<ConversationAcceptance>()]
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(sends[0].promise).mockReturnValueOnce(sends[1].promise)
            const setPanelError = vi.fn()
            const second = fileOf('2')
            // Each of two new chats sends its first message, with a file; no list names either yet.
            const sendFirst = async (conversationId: string, text: string, file: ComposerAttachment, call: number) => {
                useStore.getState().updateProjectSessionState(projectPath, { conversationId })
                useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: text })
                useStore.getState().updateHomeConversationSession(conversationId, { attachments: [file] })
                composer({ chatDraft: text, attachments: [file], setPanelError, ensureConversationId: () => conversationId, getCurrentConversationId: openChat }).onChatComposerSubmit()
                await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(call))
            }
            await sendFirst('conversation-a', 'First', entry, 1)
            await sendFirst('conversation-b', 'Second', second, 2)

            await show(null)
            sends[0].reject(new Error('offline'))
            sends[1].reject(new Error('offline'))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(setPanelError.mock.calls).toEqual([[null], [null]])

            // Back, its projects load: one chat opens again with its own text, file and error.
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-a')
            expect(draft()).toBe('First')
            expect(attachmentsOf('conversation-a')).toEqual([entry])
            expect(setPanelError.mock.calls).toEqual([[null], [null], ['Unable to send the project chat turn.']])

            // The other opens with its own at the next showing.
            await show(null)
            await show('env-r')
            loadProjects('conversation-1')
            expect(openChat(projectPath)).toBe('conversation-b')
            expect(draft()).toBe('Second')
            expect(attachmentsOf('conversation-b')).toEqual([second])
            expect(setPanelError.mock.calls).toEqual([[null], [null], ['Unable to send the project chat turn.'], ['Unable to send the project chat turn.']])
        })

        it('leaves a newer send its pending and sending state when an older one\'s answer lands after a switch away and back', async () => {
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            await show('env-r')
            composer().onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))
            await show(null)
            await show('env-r')

            // The workspace mounted anew sends to the same chat.
            composer({ chatDraft: 'Second' }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            first.resolve(accept(0))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(pending).toMatchObject({ conversationId: 'conversation-1' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })

            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()
        })

        it('leaves a newer send in another chat of the project its pending and sending state when an older one\'s answer lands after a switch away and back', async () => {
            const first = deferred<ConversationAcceptance>()
            const second = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
            await show('env-r')
            composer().onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))
            await show(null)
            await show('env-r')

            // The workspace mounted anew sends in another chat; the project's pending turn is that send's.
            composer({ chatDraft: 'Second', ensureConversationId: () => 'conversation-2', getCurrentConversationId: () => 'conversation-2' }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            first.resolve(accept(0))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(pending).toMatchObject({ conversationId: 'conversation-2' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-2': true })

            second.resolve(accept(1))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()
        })

        it('leaves a newer send its pending and sending state when a send begun before a switch away and back goes once its settings save', async () => {
            const settings = deferred<boolean | string>()
            const newer = deferred<ConversationAcceptance>()
            const older = deferred<ConversationAcceptance>()
            vi.mocked(sendConversationTurnValidated).mockReturnValueOnce(newer.promise).mockReturnValueOnce(older.promise)
            composer({ waitForPendingSettingsSave: () => settings.promise }).onChatComposerSubmit()
            await show('env-r')
            await show(null)

            // The workspace mounted anew sends in the same chat, then the older send's settings save lands.
            composer({ chatDraft: 'Second', waitForPendingSettingsSave: async () => true }).onChatComposerSubmit()
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))
            settings.resolve(true)
            await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(2))
            expect(vi.mocked(sendConversationTurnValidated).mock.calls.map(([, request]) => request.message)).toEqual(['Second', 'Hello there'])
            older.resolve(accept(1))
            await new Promise((resolve) => setTimeout(resolve, 0))
            expect(pending).toMatchObject({ conversationId: 'conversation-1' })
            expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
            expect(pendingMessages().map((message) => message.content)).toEqual(['Second'])

            newer.resolve(accept(0))
            await vi.waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
            expect(pending).toBeNull()
        })

        it('sends here while another environment\'s chat with the same ID waits on its settings save', async () => {
            const remoteSettings = deferred<boolean>()
            vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise(() => {}))
            await show('env-r')
            composer({ waitForPendingSettingsSave: () => remoteSettings.promise }).onChatComposerSubmit()
            await show(null)
            try {
                composer({ waitForPendingSettingsSave: async () => true }).onChatComposerSubmit()
                await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))
                expect(vi.mocked(sendConversationTurnValidated).mock.calls[0][2]).toBeNull()
            } finally { remoteSettings.resolve(false) }
        })
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

describe('/model and /effort slash commands', () => {
    const projectPath = '/tmp/project'
    const discovery = {
        projectPath,
        payload: {
            providers: {
                codex: { status: 'available' as const, error: null, label: 'Codex' },
                'claude-code': { status: 'available' as const, error: null, label: 'Claude Code' },
                anthropic: { status: 'available' as const, error: null, label: 'Anthropic' },
            },
            models: [
                {
                    provider: 'codex',
                    id: 'gpt-5.4',
                    display: 'GPT-5.4',
                    is_default: true,
                    supported_reasoning_efforts: ['low', 'medium', 'high'],
                    default_reasoning_effort: 'medium',
                    supported_reasoning_summaries: ['auto', 'detailed'],
                },
                {
                    provider: 'codex',
                    id: 'gpt-5.4-mini',
                    display: 'GPT-5.4 Mini',
                    is_default: false,
                    supported_reasoning_efforts: ['low', 'medium'],
                },
                {
                    provider: 'claude-code',
                    id: 'claude-opus-4-6',
                    display: 'Claude Opus 4.6',
                    is_default: true,
                    supported_reasoning_efforts: [],
                },
                {
                    provider: 'anthropic',
                    id: 'claude-opus-4-6',
                    display: 'Claude Opus 4.6',
                    is_default: false,
                    supported_reasoning_efforts: ['low', 'high'],
                },
            ],
        },
    }

    const createComposer = (overrides: Partial<Parameters<typeof useConversationComposer>[0]> = {}) => {
        const setChatDraft = vi.fn()
        const setPanelError = vi.fn()
        const onModelSettingsChange = vi.fn()
        const onOpenModelPicker = vi.fn()
        const result = useConversationComposer({
            activeProjectPath: projectPath,
            chatDraft: '',
            attachments: [],
            isUploadBlocking: false,
            isChatInputDisabled: false,
            modelSettings: { provider: null, llm_profile: null, model: null, reasoning_effort: null },
            inheritedModelSettings: { provider: 'codex', llm_profile: null, model: 'gpt-5.4', reasoning_effort: null },
            modelDiscovery: discovery,
            llmProfiles: [],
            onModelSettingsChange,
            onOpenModelPicker,
            ensureConversationId: () => 'conversation-1',
            getCurrentConversationId: () => 'conversation-1',
            getConversationRecord: () => record(),
            applyConversationBatch: vi.fn(),
            formatErrorMessage: (_error, fallback) => fallback,
            setChatDraft,
            setPanelError,
            setPendingConversationTurn: vi.fn(),
            ...overrides,
        })
        return { ...result, setChatDraft, setPanelError, onModelSettingsChange, onOpenModelPicker }
    }

    beforeEach(() => {
        useStore.setState(useStore.getInitialState(), true)
        vi.mocked(sendConversationTurnValidated).mockReset()
        vi.mocked(updateConversationSettingsValidated).mockReset()
    })

    it('opens the Model picker on bare /model without sending a message', async () => {
        const c = createComposer({ chatDraft: '/model' })
        c.onChatComposerSubmit()
        await Promise.resolve()
        expect(c.onOpenModelPicker).toHaveBeenCalledOnce()
        expect(c.setChatDraft).toHaveBeenCalledWith('')
        expect(c.setPanelError).toHaveBeenCalledWith(null)
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('selects a unique model by exact id, display name, or provider/model and records recent', async () => {
        // Exact id beats prefix overlap (gpt-5.4 vs gpt-5.4-mini).
        const byId = createComposer({ chatDraft: '/model gpt-5.4' })
        byId.onChatComposerSubmit()
        await Promise.resolve()
        expect(byId.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'codex', llm_profile: null, model: 'gpt-5.4',
        }))
        expect(byId.setChatDraft).toHaveBeenCalledWith('')
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        // Unique display name / substring match.
        const byDisplay = createComposer({ chatDraft: '/model GPT-5.4 Mini' })
        byDisplay.onChatComposerSubmit()
        await Promise.resolve()
        expect(byDisplay.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'codex', llm_profile: null, model: 'gpt-5.4-mini',
        }))

        // provider/model disambiguates identical model ids across providers.
        const byProviderModel = createComposer({ chatDraft: '/model anthropic/claude-opus-4-6' })
        byProviderModel.onChatComposerSubmit()
        await Promise.resolve()
        expect(byProviderModel.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'anthropic', llm_profile: null, model: 'claude-opus-4-6',
        }))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('surfaces clear inline errors on ambiguous or missing /model queries without sending', async () => {
        const ambiguous = createComposer({ chatDraft: '/model claude-opus-4-6' })
        ambiguous.onChatComposerSubmit()
        await Promise.resolve()
        expect(ambiguous.onModelSettingsChange).not.toHaveBeenCalled()
        expect(ambiguous.setChatDraft).not.toHaveBeenCalled()
        expect(ambiguous.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/Multiple models match "claude-opus-4-6"/))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        const missing = createComposer({ chatDraft: '/model nonexistent-model' })
        missing.onChatComposerSubmit()
        await Promise.resolve()
        expect(missing.onModelSettingsChange).not.toHaveBeenCalled()
        expect(missing.setPanelError).toHaveBeenCalledWith('No available model matches "nonexistent-model".')
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('handles /effort <level|default>, auto-pinning inherited effective model and validating supported levels', async () => {
        // Inherited effective model (codex/gpt-5.4): /effort high atomically pins model + effort.
        const pinInherited = createComposer({ chatDraft: '/effort high' })
        pinInherited.onChatComposerSubmit()
        await Promise.resolve()
        expect(pinInherited.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'codex', llm_profile: null, model: 'gpt-5.4', reasoning_effort: 'high',
        }))
        expect(pinInherited.setChatDraft).toHaveBeenCalledWith('')
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        // Explicit model: /effort default clears reasoning_effort.
        const clearExplicit = createComposer({
            chatDraft: '/effort default',
            modelSettings: { provider: 'codex', llm_profile: null, model: 'gpt-5.4', reasoning_effort: 'high' },
        })
        clearExplicit.onChatComposerSubmit()
        await Promise.resolve()
        expect(clearExplicit.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'codex', model: 'gpt-5.4', reasoning_effort: null,
        }))

        // Unsupported effort level surfaces a clear error and does not send.
        const unsupported = createComposer({ chatDraft: '/effort xhigh' })
        unsupported.onChatComposerSubmit()
        await Promise.resolve()
        expect(unsupported.onModelSettingsChange).not.toHaveBeenCalled()
        expect(unsupported.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/not supported.*low, medium, high/i))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        // Bare /effort surfaces usage error and does not send.
        const bare = createComposer({ chatDraft: '/effort' })
        bare.onChatComposerSubmit()
        await Promise.resolve()
        expect(bare.onModelSettingsChange).not.toHaveBeenCalled()
        expect(bare.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/Specify an effort level/i))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        // Unknown inherited effective model surfaces error.
        const unknown = createComposer({
            chatDraft: '/effort high',
            inheritedModelSettings: undefined,
        })
        unknown.onChatComposerSubmit()
        await Promise.resolve()
        expect(unknown.onModelSettingsChange).not.toHaveBeenCalled()
        expect(unknown.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/No active model is resolved/i))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('still allows /model recovery when chat submission is disabled due to unavailable current model', async () => {
        const recover = createComposer({
            chatDraft: '/model anthropic/claude-opus-4-6',
            isChatInputDisabled: true,
            isModelSettingsLocked: false,
        })
        recover.onChatComposerSubmit()
        await Promise.resolve()
        expect(recover.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: 'anthropic', model: 'claude-opus-4-6',
        }))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('rejects unconfigured profiles in /model <query> with inline credentials feedback and no settings write', async () => {
        const unconfigured = createComposer({
            chatDraft: '/model missing-team/secret-model',
            llmProfiles: [
                { id: 'missing-team', label: 'Missing Team', provider: 'openai_compatible', configured: false, models: ['secret-model'] },
            ],
        })
        unconfigured.onChatComposerSubmit()
        await Promise.resolve()
        expect(unconfigured.onModelSettingsChange).not.toHaveBeenCalled()
        expect(unconfigured.setChatDraft).not.toHaveBeenCalled()
        expect(unconfigured.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/Missing Team.*missing required credentials/i))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('matches available profile models by discovery display name and provider/model when unique, with actionable ambiguity feedback', async () => {
        const profileDiscovery = {
            projectPath,
            payload: {
                providers: {
                    codex: { status: 'available' as const, error: null, label: 'Codex' },
                },
                models: [
                    {
                        llm_profile: 'local',
                        provider: 'openai_compatible',
                        id: 'local-model',
                        display: 'Local / local-model',
                        is_default: true,
                        supported_reasoning_efforts: [],
                    },
                    {
                        llm_profile: 'alpha',
                        provider: 'openai_compatible',
                        id: 'shared-model',
                        display: 'Alpha / shared-model',
                        is_default: false,
                        supported_reasoning_efforts: [],
                    },
                    {
                        llm_profile: 'beta',
                        provider: 'openai_compatible',
                        id: 'shared-model',
                        display: 'Beta / shared-model',
                        is_default: false,
                        supported_reasoning_efforts: [],
                    },
                ],
            },
        }
        const profiles = [
            { id: 'local', label: 'Local', provider: 'openai_compatible', configured: true, models: ['local-model'], default_model: 'local-model' },
            { id: 'alpha', label: 'Alpha', provider: 'openai_compatible', configured: true, models: ['shared-model'] },
            { id: 'beta', label: 'Beta', provider: 'openai_compatible', configured: true, models: ['shared-model'] },
        ]

        // Match by discovery display name ("Local / local-model").
        const byDisplay = createComposer({
            chatDraft: '/model Local / local-model',
            modelDiscovery: profileDiscovery,
            llmProfiles: profiles,
        })
        byDisplay.onChatComposerSubmit()
        await Promise.resolve()
        expect(byDisplay.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: null, llm_profile: 'local', model: 'local-model',
        }))

        // Match by provider/model ("openai_compatible/local-model") when unique.
        const byProviderModel = createComposer({
            chatDraft: '/model openai_compatible/local-model',
            modelDiscovery: profileDiscovery,
            llmProfiles: profiles,
        })
        byProviderModel.onChatComposerSubmit()
        await Promise.resolve()
        expect(byProviderModel.onModelSettingsChange).toHaveBeenCalledWith(expect.objectContaining({
            provider: null, llm_profile: 'local', model: 'local-model',
        }))

        // Ambiguous provider/model across multiple profiles reports actionable profile/model selectors.
        const ambiguous = createComposer({
            chatDraft: '/model openai_compatible/shared-model',
            modelDiscovery: profileDiscovery,
            llmProfiles: profiles,
        })
        ambiguous.onChatComposerSubmit()
        await Promise.resolve()
        expect(ambiguous.onModelSettingsChange).not.toHaveBeenCalled()
        expect(ambiguous.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/alpha\/shared-model.*beta\/shared-model/))
        expect(ambiguous.setPanelError).toHaveBeenCalledWith(expect.stringMatching(/\/model alpha\/shared-model/))
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('preserves inherited Thinking, Mode, Summary, and ACP session controls when /effort pins the inherited model', async () => {
        const inherited = {
            provider: 'codex',
            llm_profile: null,
            model: 'gpt-5.4',
            reasoning_effort: 'low',
            thinking: 'budget',
            thinking_budget_tokens: 4096,
            reasoning_mode: 'pro',
            reasoning_summary: 'detailed',
            session_config: { Approval: 'auto' },
        }
        const c = createComposer({
            chatDraft: '/effort high',
            inheritedModelSettings: inherited,
        })
        c.onChatComposerSubmit()
        await Promise.resolve()
        expect(c.onModelSettingsChange).toHaveBeenCalledWith({
            ...inherited,
            reasoning_effort: 'high',
        })
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
    })

    it('waits for pending settings saves before sending a turn and aborts without sending if the save fails', async () => {
        let resolveSave!: (ok: boolean) => void
        const waitForPendingSettingsSave = vi.fn(() => new Promise<boolean>((resolve) => { resolveSave = resolve }))
        vi.mocked(sendConversationTurnValidated).mockReturnValue(new Promise(() => {}))

        const c = createComposer({
            chatDraft: 'Run with updated effort',
            waitForPendingSettingsSave,
        })
        c.onChatComposerSubmit()
        await Promise.resolve()
        // Competing send does not fire while settings save is still in flight.
        expect(waitForPendingSettingsSave).toHaveBeenCalledWith('conversation-1')
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()

        // Duplicate submit while waiting is ignored.
        c.onChatComposerSubmit()
        await Promise.resolve()
        expect(waitForPendingSettingsSave).toHaveBeenCalledTimes(1)

        // Once the settings save commits, the turn sends.
        resolveSave(true)
        await vi.waitFor(() => expect(sendConversationTurnValidated).toHaveBeenCalledTimes(1))
        expect(vi.mocked(sendConversationTurnValidated).mock.calls[0][1].message).toBe('Run with updated effort')

        // If a pending settings save fails, abort the send without clearing the settings error.
        vi.mocked(sendConversationTurnValidated).mockClear()
        const failing = createComposer({
            chatDraft: 'Should not send',
            waitForPendingSettingsSave: vi.fn().mockResolvedValue(false),
        })
        failing.onChatComposerSubmit()
        await Promise.resolve()
        await Promise.resolve()
        expect(sendConversationTurnValidated).not.toHaveBeenCalled()
        expect(failing.setPanelError).not.toHaveBeenCalledWith(null)
    })
})
