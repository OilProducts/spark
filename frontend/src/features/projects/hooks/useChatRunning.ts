import { useEffect } from 'react'
import { ApiHttpError, fetchConversationSnapshotValidated, type ConversationTurnResponse } from '@/lib/workspaceClient'
import { useStore } from '@/store'

const BACKGROUND_CHAT_POLL_MS = 2_000

const isRunningTurn = (turn: ConversationTurnResponse) => turn.status === 'pending' || turn.status === 'streaming'

/**
 * Whether any chat, in any project, has a turn in flight. The open chat follows
 * its live stream; a chat left mid-turn is checked against the server until its turn ends.
 */
// ponytail: only chats seen running in this tab are followed; a cross-project turn feed would also cover other tabs.
export function useChatRunning() {
    const openChat = useStore((state) => {
        const openChatId = state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath]?.conversationId : null
        return openChatId ? state.homeConversationCache.conversationsById[openChatId] : undefined
    })
    const runningChats = useStore((state) => state.runningChats)
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

    // Chats left mid-turn: poll their snapshots until no turn is in flight.
    const backgroundIds = Object.entries(runningChats)
        .filter(([id, chat]) => id !== openChat?.conversation_id && !chat.sending)
        .map(([id]) => id)
        .join(',')
    useEffect(() => {
        if (!backgroundIds) return
        const timer = window.setInterval(() => {
            for (const id of backgroundIds.split(',')) {
                const chat = useStore.getState().runningChats[id]
                if (!chat) continue
                fetchConversationSnapshotValidated(id, chat.projectPath).then((snapshot) => {
                    const current = useStore.getState().runningChats[id]
                    if (!current || current.sending) return
                    if (!snapshot.turns.some(isRunningTurn)) useStore.getState().setRunningChat(id, null)
                    else if (snapshot.revision > current.revision) useStore.getState().setRunningChat(id, { ...current, revision: snapshot.revision })
                }).catch((error: unknown) => {
                    // A deleted chat runs nothing; other failures retry on the next poll.
                    if (error instanceof ApiHttpError && error.status === 404) useStore.getState().setRunningChat(id, null)
                })
            }
        }, BACKGROUND_CHAT_POLL_MS)
        return () => window.clearInterval(timer)
    }, [backgroundIds])

    return openRunning || Object.keys(runningChats).length > 0
}
