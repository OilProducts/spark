import type { ProjectSessionState } from '@/store'
import type { UiDefaults } from '@/state/store-types'
import type {
    ConversationChatMode,
    ConversationSummaryResponse,
} from '@/lib/workspaceClient'

import type { PendingConversationTurnState } from './conversationState'
import type { ProjectGitMetadata } from './presentation'
import {
    EMPTY_PROJECT_GIT_METADATA,
    formatProjectListLabel,
    getConversationFlowLaunches,
    getConversationFlowRunRequests,
    getConversationProposedPlans,
    getConversationTimelineEntries,
    type NormalizedConversationRecord,
    type ProjectConversationCacheState,
} from './projectsHomeState'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
    ProjectProposedPlan,
} from './types'

type BuildProjectsHomeViewModelArgs = {
    activeConversationId: string | null
    activeConversationRecord: NormalizedConversationRecord | null
    activeProjectPath: string | null
    activeProjectScope: ProjectSessionState | null
    conversationCache: ProjectConversationCacheState
    pendingConversationTurn: PendingConversationTurnState | null
    projectGitMetadata: Record<string, ProjectGitMetadata>
    uiDefaults: UiDefaults
}

export type ProjectsHomeViewModel = {
    activeChatMode: ConversationChatMode | null
    activeProjectChatProvider: string
    activeProjectChatModel: string
    activeProjectChatReasoningEffort: string
    activeConversationHistory: ConversationTimelineEntry[]
    activeFlowLaunchesById: Map<string, ProjectFlowLaunch>
    activeFlowRunRequestsById: Map<string, ProjectFlowRunRequest>
    activeProposedPlansById: Map<string, ProjectProposedPlan>
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
    activeConversationRecord,
    activeProjectPath,
    activeProjectScope,
    conversationCache,
    pendingConversationTurn,
    projectGitMetadata,
    uiDefaults,
}: BuildProjectsHomeViewModelArgs): ProjectsHomeViewModel {
    const activeConversationHistory = getConversationTimelineEntries(activeConversationRecord)
    const activeFlowRunRequests = getConversationFlowRunRequests(activeConversationRecord)
    const activeFlowLaunches = getConversationFlowLaunches(activeConversationRecord)
    const activeProposedPlans = getConversationProposedPlans(activeConversationRecord)
    const hasRenderableConversationHistory = activeConversationHistory.some((entry) => (
        entry.kind === 'mode_change'
        || entry.kind === 'context_compaction'
        || entry.kind === 'request_user_input'
        || entry.kind === 'flow_run_request'
        || entry.kind === 'flow_launch'
        || entry.kind === 'tool_call'
        || entry.role === 'user'
        || entry.role === 'assistant'
    ))
    const hasActiveAssistantTurn = (activeConversationRecord?.orderedTurnIds || []).some((turnId) => {
        const turn = activeConversationRecord?.turnsById[turnId]
        return Boolean(
            turn && turn.role === 'assistant' && (turn.status === 'pending' || turn.status === 'streaming'),
        )
    })
    const isConversationTurnStarting = Boolean(
        pendingConversationTurn
            && pendingConversationTurn.conversationId === activeConversationId
            && (!activeConversationRecord || activeConversationRecord.revision <= pendingConversationTurn.afterRevision),
    )

    return {
        activeChatMode: activeConversationId
            ? (activeConversationRecord?.chat_mode ?? 'chat')
            : null,
        activeProjectChatModel: (
            activeConversationRecord?.model
            ?? uiDefaults.llm_model
            ?? ''
        ),
        activeProjectChatProvider: (
            activeConversationRecord?.provider
            ?? uiDefaults.llm_provider
            ?? 'codex'
        ),
        activeProjectChatReasoningEffort: (
            activeConversationRecord?.reasoning_effort
            ?? uiDefaults.reasoning_effort
            ?? ''
        ),
        activeConversationHistory,
        activeFlowLaunchesById: buildIdMap(activeFlowLaunches),
        activeFlowRunRequestsById: buildIdMap(activeFlowRunRequests),
        activeProposedPlansById: buildIdMap(activeProposedPlans),
        activeProjectConversationSummaries: activeProjectPath
            ? conversationCache.summariesByProjectPath[activeProjectPath] || []
            : [],
        activeProjectEventLog: activeProjectScope?.projectEventLog || [],
        activeProjectGitMetadata: activeProjectPath
            ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
            : EMPTY_PROJECT_GIT_METADATA,
        activeProjectLabel: activeProjectPath ? formatProjectListLabel(activeProjectPath) : null,
        chatSendButtonLabel: isConversationTurnStarting ? 'Sending...' : hasActiveAssistantTurn ? 'Thinking...' : 'Send',
        hasActiveAssistantTurn,
        hasRenderableConversationHistory,
        isChatInputDisabled: isConversationTurnStarting || hasActiveAssistantTurn,
        latestFlowLaunchId: getLatestArtifactId(activeFlowLaunches),
        latestFlowRunRequestId: getLatestArtifactId(activeFlowRunRequests),
    }
}
