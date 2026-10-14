import type { Dispatch, FormEvent, KeyboardEvent, SetStateAction } from 'react'
import {
    retryConversationTurnValidated,
    sendConversationTurnValidated,
    type ConversationChatMode,
    type ConversationSnapshotResponse,
    updateConversationSettingsValidated,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import type { PendingConversationTurnState } from '../model/conversationState'
import type { ComposerAttachment } from '../model/types'
import { clearSentAttachments } from './useComposerAttachments'
import { reconcileRunningChat } from './useChatRunning'

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
    getCurrentConversationRevision: (conversationId: string) => number
    applyConversationSnapshot: (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
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
    getCurrentConversationRevision,
    applyConversationSnapshot,
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
            const snapshot = await updateConversationSettingsValidated(conversationId, {
                project_path: activeProjectPath,
                chat_mode: chatMode,
                expected_revision: String(getCurrentConversationRevision(conversationId)),
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'settings-response', {
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
        const attachmentIds = sending.map((entry) => entry.attachment!.id)
        await runTurn(conversationId, 'Unable to send the project chat turn.', () => sendConversationTurnValidated(conversationId, {
            project_path: activeProjectPath,
            message: messageToSend,
            ...(attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : {}),
            chat_mode: chatMode,
        }), () => {
            // Accepted: clear what was sent, keeping anything typed or attached meanwhile.
            clearSentAttachments(conversationId, sending.map((entry) => entry.localId))
            const store = useStore.getState()
            if (store.homeProjectSessionsByPath[activeProjectPath]?.chatDraft === chatDraft) {
                store.updateHomeProjectSession(activeProjectPath, { chatDraft: '' })
            }
        })
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
        start: () => Promise<ConversationSnapshotResponse>,
        onAccepted?: () => void,
    ) => {
        if (!activeProjectPath) {
            return
        }
        const afterRevision = getCurrentConversationRevision(conversationId)
        setPendingConversationTurn({ conversationId, afterRevision })
        // The activity bar follows the turn even after the chat is left.
        const { setRunningChat } = useStore.getState()
        setRunningChat(conversationId, { projectPath: activeProjectPath, revision: afterRevision, sending: true })
        try {
            const snapshot = await start()
            onAccepted?.()
            setPendingConversationTurn(null)
            const shouldKeepFocusOnReplyThread = getCurrentConversationId(activeProjectPath) === conversationId
            applyConversationSnapshot(activeProjectPath, snapshot, 'send-response', {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
        } catch (error) {
            const message = formatErrorMessage(error, failure)
            setPanelError(message)
            // A rejected send may still have started a turn; the server decides whether the chat runs.
            void reconcileRunningChat(conversationId, activeProjectPath)
        } finally {
            setPendingConversationTurn(null)
            const running = useStore.getState().runningChats[conversationId]
            if (running) setRunningChat(conversationId, { ...running, sending: false })
        }
    }

    const onChatComposerSubmit = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        void onSendChatMessage()
    }

    const onChatComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
        if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault()
            void onSendChatMessage()
        }
    }

    return {
        onChatComposerKeyDown,
        onChatComposerSubmit,
        onRetryTurn,
        onSwitchChatMode,
    }
}
