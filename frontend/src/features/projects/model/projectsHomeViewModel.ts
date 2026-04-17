import type { ProjectSessionState } from '@/store'
import type {
    ConversationChatMode,
    ConversationSnapshotResponse,
    ConversationSummaryResponse,
} from '@/lib/workspaceClient'

import type { OptimisticSendState } from './conversationState'
import { buildConversationTimelineEntries } from './conversationTimeline'
import type { ProjectGitMetadata } from './presentation'
import {
    EMPTY_PROJECT_GIT_METADATA,
    formatProjectListLabel,
    type ProjectConversationCacheState,
} from './projectsHomeState'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
} from './types'

type BuildProjectsHomeViewModelArgs = {
    activeConversationId: string | null
    activeConversationSnapshot: ConversationSnapshotResponse | null
    activeProjectPath: string | null
    activeProjectScope: ProjectSessionState | null
    conversationCache: ProjectConversationCacheState
    optimisticSend: OptimisticSendState | null
    projectGitMetadata: Record<string, ProjectGitMetadata>
}

export type ProjectsHomeViewModel = {
    activeChatMode: ConversationChatMode | null
    activeConversationHistory: ConversationTimelineEntry[]
    activeFlowLaunchesById: Map<string, ProjectFlowLaunch>
    activeFlowRunRequestsById: Map<string, ProjectFlowRunRequest>
    activeProjectConversationSummaries: ConversationSummaryResponse[]
    activeProjectEventLog: ProjectSessionState['projectEventLog']
    activeProjectGitMetadata: ProjectGitMetadata
    activeProjectLabel: string | null
    chatSendButtonLabel: string
    hasActiveAssistantTurn: boolean
    hasRenderableConversationHistory: boolean
    isChatInputDisabled: boolean
    latestFlowLaunchId: string | null
    latestFlowRunRequestId: string | null
}

function buildIdMap<T extends { id: string }>(items: T[]) {
    return new Map(items.map((item) => [item.id, item]))
}

function getLatestArtifactId<T extends { id: string }>(items: T[]) {
    return items.length > 0 ? items[items.length - 1]?.id || null : null
}

export function buildProjectsHomeViewModel({
    activeConversationId,
    activeConversationSnapshot,
    activeProjectPath,
    activeProjectScope,
    conversationCache,
    optimisticSend,
    projectGitMetadata,
}: BuildProjectsHomeViewModelArgs): ProjectsHomeViewModel {
    const activeConversationHistory = buildConversationTimelineEntries(
        activeConversationSnapshot,
        optimisticSend && optimisticSend.conversationId === activeConversationId ? optimisticSend : null,
    )
    const activeFlowRunRequests = activeConversationSnapshot?.flow_run_requests || []
    const activeFlowLaunches = activeConversationSnapshot?.flow_launches || []
    const hasRenderableConversationHistory = activeConversationHistory.some((entry) => (
        entry.kind === 'mode_change'
        || entry.kind === 'context_compaction'
        || entry.kind === 'flow_run_request'
        || entry.kind === 'flow_launch'
        || entry.kind === 'tool_call'
        || entry.role === 'user'
        || entry.role === 'assistant'
    ))
    const hasActiveAssistantTurn = (activeConversationSnapshot?.turns || []).some((turn) => (
        turn.role === 'assistant' && (turn.status === 'pending' || turn.status === 'streaming')
    ))

    return {
        activeChatMode: activeConversationId
            ? (activeConversationSnapshot?.chat_mode ?? 'chat')
            : null,
        activeConversationHistory,
        activeFlowLaunchesById: buildIdMap(activeFlowLaunches),
        activeFlowRunRequestsById: buildIdMap(activeFlowRunRequests),
        activeProjectConversationSummaries: activeProjectPath
            ? conversationCache.summariesByProjectPath[activeProjectPath] || []
            : [],
        activeProjectEventLog: activeProjectScope?.projectEventLog || [],
        activeProjectGitMetadata: activeProjectPath
            ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
            : EMPTY_PROJECT_GIT_METADATA,
        activeProjectLabel: activeProjectPath ? formatProjectListLabel(activeProjectPath) : null,
        chatSendButtonLabel: hasActiveAssistantTurn ? 'Thinking...' : 'Send',
        hasActiveAssistantTurn,
        hasRenderableConversationHistory,
        isChatInputDisabled: hasActiveAssistantTurn,
        latestFlowLaunchId: getLatestArtifactId(activeFlowLaunches),
        latestFlowRunRequestId: getLatestArtifactId(activeFlowRunRequests),
    }
}
