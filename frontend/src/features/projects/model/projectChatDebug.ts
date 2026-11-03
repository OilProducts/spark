import type { ConversationTurnResponse } from '@/lib/workspaceClient'

export const isProjectChatDebugEnabled = () => {
    if (typeof window === 'undefined') {
        return false
    }
    try {
        const params = new URLSearchParams(window.location.search)
        if (params.get('debugProjectChat') === '1') {
            return true
        }
        return window.localStorage.getItem('spark.debug.project_chat') === '1'
    } catch {
        return false
    }
}

export const summarizeConversationTurnsForDebug = (turns: ConversationTurnResponse[]) => (
    turns.map((turn, index) => ({
        index,
        id: turn.id,
        role: turn.role,
        kind: turn.kind,
        status: turn.status,
        artifactId: turn.artifact_id ?? null,
        content: turn.content.slice(0, 120),
    }))
)

export const debugProjectChat = (message: string, details?: Record<string, unknown>) => {
    if (!isProjectChatDebugEnabled()) {
        return
    }
    if (details) {
        console.debug(`[project-chat] ${message}`, details)
        return
    }
    console.debug(`[project-chat] ${message}`)
}
