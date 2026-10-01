import { useEffect } from 'react'
import {
    ApiHttpError,
    fetchConversationSnapshotValidated,
    fetchProjectConversationListValidated,
    type ConversationTurnResponse,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'

const CHAT_POLL_MS = 3_000

const isRunningTurn = (turn: ConversationTurnResponse) => turn.status === 'pending' || turn.status === 'streaming'

/**
 * Checks one chat against the server and tracks it while a turn is in flight there.
 * Resolves whether the check succeeded (a missing chat counts as checked).
 */
export async function reconcileRunningChat(conversationId: string, projectPath: string): Promise<boolean> {
    try {
        const snapshot = await fetchConversationSnapshotValidated(conversationId, projectPath)
        const { runningChats, setRunningChat } = useStore.getState()
        const current = runningChats[conversationId]
        // A send in flight, or a newer revision seen since this fetch started, wins.
        if (current && (current.sending || current.revision > snapshot.revision)) return true
        if (snapshot.turns.some(isRunningTurn)) {
            setRunningChat(conversationId, { projectPath, revision: snapshot.revision, sending: false })
        } else if (current) {
            setRunningChat(conversationId, null)
        }
        return true
    } catch (error) {
        // A deleted (or never created) chat runs nothing; other failures retry on the next poll.
        if (!(error instanceof ApiHttpError && error.status === 404)) return false
        const current = useStore.getState().runningChats[conversationId]
        if (current && !current.sending) useStore.getState().setRunningChat(conversationId, null)
        return true
    }
}

/**
 * Whether any chat, in any project, has a turn in flight. The open chat follows
 * its live stream; every registered project's chat list is polled, and chats that
 * changed, or are tracked as running, are checked against the server.
 */
// ponytail: polls each project's chat list; a cross-project turn feed on the live stream would replace it.
export function useChatRunning() {
    const openChat = useStore((state) => {
        const openChatId = state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath]?.conversationId : null
        return openChatId ? state.homeConversationCache.conversationsById[openChatId] : undefined
    })
    const runningChats = useStore((state) => state.runningChats)
    const projectPaths = useStore((state) => Object.keys(state.projectRegistry).sort().join('\n'))
    const openRunning = Object.values(openChat?.turnsById ?? {}).some(isRunningTurn)

    // The open chat: follow it while it runs, and drop it once a later revision shows it done.
    useEffect(() => {
        if (!openChat) return
        const id = openChat.conversation_id
        const tracked = runningChats[id]
        if (openRunning) {
            if (!tracked || tracked.revision < openChat.revision) {
                useStore.getState().setRunningChat(id, { projectPath: openChat.project_path, revision: openChat.revision, sending: tracked?.sending ?? false })
            }
        } else if (tracked && !tracked.sending && openChat.revision > tracked.revision) {
            useStore.getState().setRunningChat(id, null)
        }
    }, [openChat, openRunning, runningChats])

    // Every project: find chats started elsewhere (another tab, before a reload) and see tracked ones finish.
    useEffect(() => {
        const seenRevisions = new Map<string, number>()
        let timer: number | undefined
        let stopped = false
        const poll = async () => {
            // Chat id -> its project, and the listed revision to remember once the check succeeds.
            const checks = new Map<string, { projectPath: string, revision?: number }>()
            for (const [id, chat] of Object.entries(useStore.getState().runningChats)) {
                if (!chat.sending) checks.set(id, { projectPath: chat.projectPath })
            }
            await Promise.all(projectPaths.split('\n').filter(Boolean).map(async (projectPath) => {
                try {
                    const summaries = await fetchProjectConversationListValidated(projectPath)
                    // ponytail: the first poll checks every chat once; a running flag on summaries would avoid it.
                    for (const summary of summaries) {
                        if (seenRevisions.get(summary.conversation_id) !== summary.revision) {
                            checks.set(summary.conversation_id, { projectPath, revision: summary.revision })
                        }
                    }
                } catch {
                    // Retried on the next poll.
                }
            }))
            await Promise.all([...checks].map(async ([id, { projectPath, revision }]) => {
                if (await reconcileRunningChat(id, projectPath) && revision !== undefined) seenRevisions.set(id, revision)
            }))
            if (!stopped) timer = window.setTimeout(poll, CHAT_POLL_MS)
        }
        void poll()
        return () => {
            stopped = true
            window.clearTimeout(timer)
        }
    }, [projectPaths])

    return openRunning || Object.keys(runningChats).length > 0
}
