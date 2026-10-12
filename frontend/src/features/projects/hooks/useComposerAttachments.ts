import { useCallback } from 'react'
import {
    CONVERSATION_ATTACHMENT_LIMITS,
    deleteConversationAttachmentValidated,
    uploadConversationAttachmentValidated,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { extractApiErrorMessage } from '../model/projectsHomeState'
import type { ComposerAttachment } from '../model/types'

export { CONVERSATION_ATTACHMENT_LIMITS }

const EMPTY: ComposerAttachment[] = []
const { maxFiles, maxFileBytes, maxMessageBytes } = CONVERSATION_ATTACHMENT_LIMITS

/** Images the server serves inline and the providers take natively. */
export const PREVIEWABLE_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

function readDraft(conversationId: string): ComposerAttachment[] {
    return useStore.getState().homeConversationSessionsById[conversationId]?.attachments ?? EMPTY
}

function updateDraft(conversationId: string, update: (current: ComposerAttachment[]) => ComposerAttachment[]) {
    useStore.getState().updateHomeConversationSession(conversationId, { attachments: update(readDraft(conversationId)) })
}

function setEntry(conversationId: string, localId: string, patch: Partial<ComposerAttachment>): boolean {
    let found = false
    updateDraft(conversationId, (current) => current.map((entry) => {
        if (entry.localId !== localId) return entry
        found = true
        return { ...entry, ...patch }
    }))
    return found
}

async function upload(conversationId: string, entry: ComposerAttachment) {
    try {
        const attachment = await uploadConversationAttachmentValidated(conversationId, entry.projectPath, entry.file)
        // Removed while uploading: the server copy is unwanted too.
        if (!setEntry(conversationId, entry.localId, { status: 'uploaded', attachment, error: null })) {
            void deleteConversationAttachmentValidated(conversationId, attachment.id, entry.projectPath).catch(() => undefined)
        }
    } catch (error) {
        setEntry(conversationId, entry.localId, {
            status: 'failed',
            error: extractApiErrorMessage(error, `Unable to upload ${entry.file.name}.`),
        })
    }
}

function release(entries: ComposerAttachment[]) {
    entries.forEach((entry) => {
        if (entry.previewUrl) URL.revokeObjectURL(entry.previewUrl)
    })
}

export function formatAttachmentSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/**
 * The active chat's unsent attachments. Paste, drop and the Attach button all
 * call `addFiles`; drafts live per conversation in the store.
 */
export function useComposerAttachments({
    activeConversationId,
    activeProjectPath,
    ensureConversationId,
    setPanelError,
}: {
    activeConversationId: string | null
    activeProjectPath: string | null
    ensureConversationId: () => string | null
    setPanelError: (message: string | null) => void
}) {
    const attachments = useStore((state) => (
        activeConversationId ? state.homeConversationSessionsById[activeConversationId]?.attachments ?? EMPTY : EMPTY
    ))

    const addFiles = useCallback((files: File[]) => {
        if (!activeProjectPath || files.length === 0) return
        const conversationId = ensureConversationId()
        if (!conversationId) return
        const current = readDraft(conversationId)
        let total = current.reduce((sum, entry) => sum + entry.file.size, 0)
        const accepted: ComposerAttachment[] = []
        const problems: string[] = []
        for (const file of files) {
            if (current.length + accepted.length >= maxFiles) {
                problems.push(`A message can carry at most ${maxFiles} attachments.`)
                break
            }
            if (file.size > maxFileBytes) {
                problems.push(`${file.name} is larger than the 20 MB per-file limit.`)
            } else if (total + file.size > maxMessageBytes) {
                problems.push(`${file.name} would take this message past its 50 MB limit.`)
            } else {
                total += file.size
                accepted.push({
                    localId: crypto.randomUUID(),
                    projectPath: activeProjectPath,
                    file,
                    previewUrl: PREVIEWABLE_IMAGE_TYPES.includes(file.type) ? URL.createObjectURL(file) : null,
                    status: 'uploading',
                    error: null,
                    attachment: null,
                })
            }
        }
        setPanelError(problems.length > 0 ? problems.join(' ') : null)
        updateDraft(conversationId, (draft) => [...draft, ...accepted])
        accepted.forEach((entry) => void upload(conversationId, entry))
    }, [activeProjectPath, ensureConversationId, setPanelError])

    const retry = useCallback((localId: string) => {
        if (!activeConversationId) return
        const entry = readDraft(activeConversationId).find((item) => item.localId === localId)
        if (!entry || entry.status !== 'failed') return
        setEntry(activeConversationId, localId, { status: 'uploading', error: null })
        void upload(activeConversationId, entry)
    }, [activeConversationId])

    // A send in flight still references the draft's uploads; keep them until the server answers.
    const isSending = useStore((state) => (
        activeConversationId ? Boolean(state.runningChats[activeConversationId]?.sending) : false
    ))

    const remove = useCallback((localId: string) => {
        if (!activeConversationId || useStore.getState().runningChats[activeConversationId]?.sending) return
        const removed = readDraft(activeConversationId).filter((item) => item.localId === localId)
        updateDraft(activeConversationId, (draft) => draft.filter((item) => item.localId !== localId))
        release(removed)
        removed.forEach((entry) => {
            if (entry.attachment) {
                // ponytail: a failed delete is left for the server's 24-hour cleanup.
                void deleteConversationAttachmentValidated(activeConversationId, entry.attachment.id, entry.projectPath).catch(() => undefined)
            }
        })
    }, [activeConversationId])

    return {
        attachments,
        addFiles,
        retry,
        remove,
        isSending,
        isUploadBlocking: attachments.some((entry) => entry.status !== 'uploaded'),
    }
}

/** Drops sent entries from a chat's draft; their uploads now belong to the turn. */
export function clearSentAttachments(conversationId: string, localIds: string[]) {
    const sent = readDraft(conversationId).filter((entry) => localIds.includes(entry.localId))
    updateDraft(conversationId, (draft) => draft.filter((entry) => !localIds.includes(entry.localId)))
    release(sent)
}
