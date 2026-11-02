import { useStore } from '@/store'

/**
 * Whether any chat, in any project, has a turn in flight: the open chat's own
 * turns, any conversation summary the server reports running, or a send whose
 * request is still open.
 */
export function useChatRunning() {
    return useStore((state) => {
        const openChatId = state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath]?.conversationId : null
        const openChat = openChatId ? state.homeConversationCache.conversationsById[openChatId] : undefined
        return Object.values(openChat?.turnsById ?? {}).some((turn) => turn.status === 'pending' || turn.status === 'streaming')
            || Object.values(state.homeConversationCache.summariesByProjectPath).some((summaries) => summaries.some((summary) => summary.running))
            || Object.keys(state.sendingChats).length > 0
    })
}
