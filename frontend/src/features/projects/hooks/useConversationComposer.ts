import type { Dispatch, SetStateAction } from 'react'
import {
    retryConversationTurnValidated,
    sendConversationTurnValidated,
    type ConversationAcceptance,
    type ConversationBatch,
    type ConversationChatMode,
    updateConversationSettingsValidated,
} from '@/lib/workspaceClient'
import { randomHexId } from '@/lib/utils'
import { useStore } from '@/store'
import type { PendingConversationTurnState, PendingUserMessage } from '../model/conversationState'
import type { NormalizedConversationRecord } from '../model/projectsHomeState'
import type { ComposerAttachment } from '../model/types'
import { clearSentAttachments } from './useComposerAttachments'

export type ConversationComposerCommand =
    | {
        kind: 'switch_mode'
        chatMode: ConversationChatMode
    }
    | {
        kind: 'switch_and_send'
        chatMode: ConversationChatMode
        message: string
    }

export function parseConversationComposerCommand(value: string): ConversationComposerCommand | null {
    const trimmed = value.trim()
    const match = trimmed.match(/^\/(plan|chat)(?:\s+([\s\S]*))?$/)
    if (!match) {
        return null
    }
    const chatMode = match[1] === 'plan' ? 'plan' : 'chat'
    const message = (match[2] || '').trim()
    if (!message) {
        return {
            kind: 'switch_mode',
            chatMode,
        }
    }
    return {
        kind: 'switch_and_send',
        chatMode,
        message,
    }
}

type UseConversationComposerArgs = {
    activeProjectPath: string | null
    chatDraft: string
    attachments: ComposerAttachment[]
    isUploadBlocking: boolean
    isChatInputDisabled: boolean
    ensureConversationId: () => string | null
    getCurrentConversationId: (projectPath: string) => string | null
    getConversationRecord: (conversationId: string) => NormalizedConversationRecord | undefined
    /** Merges a request's own result rows; they never move the live cursor. */
    applyConversationBatch: (
        projectPath: string,
        conversationId: string,
        batch: ConversationBatch,
        kind: 'live' | 'mutation',
        source?: string,
        options?: { forceWorkspaceSync?: boolean },
    ) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    setChatDraft: Dispatch<SetStateAction<string>>
    setPanelError: (message: string | null) => void
    setPendingConversationTurn: Dispatch<SetStateAction<PendingConversationTurnState | null>>
}

export function useConversationComposer({
    activeProjectPath,
    chatDraft,
    attachments,
    isUploadBlocking,
    isChatInputDisabled,
    ensureConversationId,
    getCurrentConversationId,
    getConversationRecord,
    applyConversationBatch,
    formatErrorMessage,
    setChatDraft,
    setPanelError,
    setPendingConversationTurn,
}: UseConversationComposerArgs) {
    // The `/plan` and `/chat` commands and the header's mode control take the same path.
    const switchChatMode = async (conversationId: string, chatMode: ConversationChatMode) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        try {
            // Changed and unchanged settings both answer with the conversation row; the reducer
            // keeps whichever of it and a live update is newer.
            const ack = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                chat_mode: chatMode,
                expected_revision: getConversationRecord(conversationId)?.settingsRevision ?? '0',
            })
            applyConversationBatch(activeProjectPath, conversationId, ack.changes, 'mutation', 'settings-response', {
                forceWorkspaceSync: true,
            })
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to switch the project chat mode.')
            setPanelError(message)
        }
    }

    const onSwitchChatMode = async (chatMode: ConversationChatMode) => {
        if (!activeProjectPath || isChatInputDisabled) {
            return
        }
        const conversationId = ensureConversationId()
        if (conversationId) {
            await switchChatMode(conversationId, chatMode)
        }
    }

    const onSendChatMessage = async () => {
        if (!activeProjectPath || isChatInputDisabled || isUploadBlocking) {
            return
        }
        const trimmed = chatDraft.trim()
        const sending = attachments.filter((entry) => entry.attachment !== null)
        if (!trimmed && sending.length === 0) {
            return
        }
        const parsedCommand = parseConversationComposerCommand(trimmed)
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }

        setPanelError(null)
        if (parsedCommand?.kind === 'switch_mode' && sending.length === 0) {
            setChatDraft('')
            await switchChatMode(conversationId, parsedCommand.chatMode)
            return
        }
        const messageToSend = parsedCommand?.kind === 'switch_and_send'
            ? parsedCommand.message
            : parsedCommand?.kind === 'switch_mode' ? '' : trimmed
        const chatMode = parsedCommand?.chatMode ?? null
        const sentAttachments = sending.map((entry) => entry.attachment!)
        const attachmentIds = sentAttachments.map((attachment) => attachment.id)
        // The server records the message under this id, so the pending entry is
        // replaced in place and a resent request cannot add a second turn.
        const clientTurnId = `turn-${randomHexId()}`
        const updatePendingMessages = (update: (messages: PendingUserMessage[]) => PendingUserMessage[]) => {
            const store = useStore.getState()
            store.updateHomeConversationSession(conversationId, {
                pendingMessages: update(store.homeConversationSessionsById[conversationId]?.pendingMessages ?? []),
            })
        }
        updatePendingMessages((messages) => [...messages, {
            id: clientTurnId,
            kind: 'message',
            role: 'user',
            content: messageToSend,
            timestamp: new Date().toISOString(),
            status: 'pending',
            ...(sentAttachments.length > 0 ? { attachments: sentAttachments } : {}),
        }])
        // The message shows in the transcript now, so the composer clears now;
        // a failed send puts the text back ahead of anything typed meanwhile.
        const currentDraft = () => useStore.getState().homeProjectSessionsByPath[activeProjectPath]?.chatDraft ?? ''
        const setDraft = (draft: string) => useStore.getState().updateHomeProjectSession(activeProjectPath, { chatDraft: draft })
        if (currentDraft() === chatDraft) {
            setDraft('')
        }
        const accepted = await runTurn(conversationId, 'Unable to send the project chat turn.', () => sendConversationTurnValidated(conversationId, {
            project_path: activeProjectPath,
            message: messageToSend,
            client_turn_id: clientTurnId,
            ...(attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : {}),
            chat_mode: chatMode,
        }), () => {
            // Accepted: clear what was sent, keeping anything attached meanwhile.
            clearSentAttachments(conversationId, sending.map((entry) => entry.localId))
        })
        if (!accepted) {
            updatePendingMessages((messages) => messages.filter((message) => message.id !== clientTurnId))
            setDraft([chatDraft, currentDraft()].filter(Boolean).join('\n\n'))
        }
    }

    /** Resends a failed turn's message as a new turn; the draft is left alone. */
    const onRetryTurn = async (turnId: string) => {
        const conversationId = activeProjectPath ? getCurrentConversationId(activeProjectPath) : null
        if (!activeProjectPath || !conversationId || isChatInputDisabled) {
            return
        }
        setPanelError(null)
        await runTurn(conversationId, 'Unable to retry the project chat turn.', () => retryConversationTurnValidated(conversationId, turnId, activeProjectPath))
    }

    /** Starts a turn through `start`, tracking it as running until the server answers. */
    const runTurn = async (
        conversationId: string,
        failure: string,
        start: () => Promise<ConversationAcceptance>,
        onAccepted?: () => void,
    ): Promise<boolean> => {
        if (!activeProjectPath) {
            return false
        }
        const afterRevision = getConversationRecord(conversationId)?.latestRevision ?? '0'
        setPendingConversationTurn({ conversationId, afterRevision })
        // The activity bar follows the turn even after the chat is left.
        const { setSendingChat } = useStore.getState()
        setSendingChat(conversationId, true)
        try {
            const acceptance = await start()
            onAccepted?.()
            setPendingConversationTurn(null)
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            // The accepted user turn carries the sent client_turn_id, so it replaces the pending
            // message in place; a resend of that id returns the same acceptance and rows.
            applyConversationBatch(activeProjectPath, conversationId, acceptance.changes, 'mutation', 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
            return true
        } catch (error) {
            const message = formatErrorMessage(error, failure)
            setPanelError(message)
            return false
        } finally {
            setPendingConversationTurn(null)
            setSendingChat(conversationId, false)
        }
    }

    const onChatComposerSubmit = () => {
        void onSendChatMessage()
    }

    return {
        onChatComposerSubmit,
        onRetryTurn,
        onSwitchChatMode,
    }
}
