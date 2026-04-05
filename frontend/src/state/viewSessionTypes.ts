import type { LaunchInputFormValues } from '@/lib/flowContracts'
import type { Edge, Node } from '@xyflow/react'
import type {
    ConversationSnapshotResponse,
    ConversationSummaryResponse,
    TriggerResponse,
} from '@/lib/workspaceClient'
import type { OptimisticSendState } from '@/features/projects/model/conversationState'
import type { ProjectGitMetadata } from '@/features/projects/model/presentation'
import type {
    ArtifactErrorState,
    ArtifactListResponse,
    CheckpointErrorState,
    CheckpointResponse,
    ContextErrorState,
    ContextResponse,
    PendingQuestionSnapshot,
    RunRecord,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
} from '@/features/runs/model/shared'
import type { TriggerFormState } from '@/features/triggers/model/triggerForm'

export type ResourceStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface HomeConversationCacheState {
    snapshotsByConversationId: Record<string, ConversationSnapshotResponse>
    summariesByProjectPath: Record<string, ConversationSummaryResponse[]>
}

export interface HomeProjectSessionState {
    chatDraft: string
    panelError: string | null
    optimisticSend: OptimisticSendState | null
    pendingDeleteConversationId: string | null
    sidebarPrimaryHeight: number
}

export interface HomeConversationSessionState {
    expandedProposalChanges: Record<string, boolean>
    expandedToolCalls: Record<string, boolean>
    expandedThinkingEntries: Record<string, boolean>
    isPinnedToBottom: boolean
    scrollTop: number | null
}

export interface RunsListSessionState {
    scopeMode: 'active' | 'all'
    selectedRunIdByScopeKey: Record<string, string | null>
    status: ResourceStatus
    error: string | null
    runs: RunRecord[]
    lastFetchedAtMs: number | null
    nowMs: number
    metadataStaleAfterMs: number
}

export interface RunDetailSessionState {
    summaryRecord: RunRecord | null
    completedNodesSnapshot: string[]
    statusFetchedAtMs: number | null
    graphStatus: ResourceStatus
    graphError: string | null
    graphNodes: Node[]
    graphEdges: Edge[]
    graphLastLayoutMs: number
    checkpointData: CheckpointResponse | null
    checkpointStatus: ResourceStatus
    checkpointError: CheckpointErrorState | null
    isSummaryCollapsed: boolean
    isActivityCollapsed: boolean
    isRawLogsCollapsed: boolean
    isTimelineCollapsed: boolean
    isCheckpointCollapsed: boolean
    isContextCollapsed: boolean
    isArtifactsCollapsed: boolean
    isGraphCollapsed: boolean
    contextData: ContextResponse | null
    contextStatus: ResourceStatus
    contextError: ContextErrorState | null
    contextSearchQuery: string
    contextCopyStatus: string
    artifactData: ArtifactListResponse | null
    artifactStatus: ResourceStatus
    artifactError: ArtifactErrorState | null
    selectedArtifactPath: string | null
    artifactViewerStatus: ResourceStatus
    artifactViewerPayload: string
    artifactViewerError: string | null
    questionsStatus: ResourceStatus
    pendingQuestionSnapshots: PendingQuestionSnapshot[]
    timelineEvents: TimelineEventEntry[]
    timelineError: string | null
    isTimelineLive: boolean
    timelineSequence: number
    timelineTypeFilter: string
    timelineNodeStageFilter: string
    timelineCategoryFilter: 'all' | TimelineEventCategory
    timelineSeverityFilter: 'all' | TimelineSeverity
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
    answeredGateIds: Record<string, boolean>
    freeformAnswersByGateId: Record<string, string>
}

export interface TriggerCreateDraftState {
    form: TriggerFormState
    targetBehavior: 'default' | 'active' | 'manual'
}

export interface TriggerEditDraftState {
    triggerId: string | null
    form: TriggerFormState | null
    targetBehavior: 'inferred' | 'active' | 'manual'
}

export interface TriggersSessionState {
    status: ResourceStatus
    error: string | null
    triggers: TriggerResponse[]
    selectedTriggerId: string | null
    scopeFilter: 'all' | 'active'
    revealedWebhookSecrets: Record<string, string>
    newTriggerDraft: TriggerCreateDraftState
    editTriggerDraftsByTriggerId: Record<string, TriggerEditDraftState>
}

export interface LaunchFailureDiagnostics {
    message: string
    failedAt: string
    flowSource: string | null
}

export interface HomeSessionSlice {
    homeConversationCache: HomeConversationCacheState
    homeThreadSummariesStatusByProjectPath: Record<string, ResourceStatus>
    homeThreadSummariesErrorByProjectPath: Record<string, string | null>
    homeProjectSessionsByPath: Record<string, HomeProjectSessionState>
    homeConversationSessionsById: Record<string, HomeConversationSessionState>
    homeProjectGitMetadataByPath: Record<string, ProjectGitMetadata>
    updateHomeProjectSession: (projectPath: string, patch: Partial<HomeProjectSessionState>) => void
    updateHomeConversationSession: (conversationId: string, patch: Partial<HomeConversationSessionState>) => void
    commitHomeConversationCache: (
        next:
            | HomeConversationCacheState
            | ((current: HomeConversationCacheState) => HomeConversationCacheState),
    ) => void
    setHomeConversationSummaryList: (
        projectPath: string,
        summaries: ConversationSummaryResponse[],
    ) => void
    setHomeThreadSummariesStatus: (
        projectPath: string,
        status: ResourceStatus,
        error?: string | null,
    ) => void
    setHomeProjectGitMetadata: (
        projectPath: string,
        metadata: ProjectGitMetadata | ((current: ProjectGitMetadata) => ProjectGitMetadata),
    ) => void
    clearHomeConversationSession: (conversationId: string) => void
    removeHomeProjectSession: (projectPath: string) => void
    renameHomeProjectSession: (currentProjectPath: string, nextProjectPath: string) => void
}

export interface RunsSessionSlice {
    runsListSession: RunsListSessionState
    runDetailSessionsByRunId: Record<string, RunDetailSessionState>
    updateRunsListSession: (patch: Partial<RunsListSessionState>) => void
    setRunsSelectedRunIdForScope: (scopeKey: string, runId: string | null) => void
    updateRunDetailSession: (runId: string, patch: Partial<RunDetailSessionState>) => void
    clearRunDetailSession: (runId: string) => void
    pruneRunDetailSessions: (runIds: string[]) => void
}

export interface TriggersSessionSlice {
    triggersSession: TriggersSessionState
    updateTriggersSession: (patch: Partial<TriggersSessionState>) => void
    setTriggersSessionNewDraft: (draft: TriggerCreateDraftState) => void
    setTriggersSessionEditDraft: (triggerId: string, draft: TriggerEditDraftState | null) => void
}

export interface ExecutionSessionSlice {
    executionLaunchInputValues: LaunchInputFormValues
    executionLaunchError: string | null
    executionLastLaunchFailure: LaunchFailureDiagnostics | null
    executionRunStartGitPolicyWarning: string | null
    executionCollapsedLaunchInputsByFlow: Record<string, boolean>
    executionGraphCollapsed: boolean
    executionOpenRunsAfterLaunch: boolean
    executionLaunchSuccessRunId: string | null
    updateExecutionSession: (patch: {
        executionLaunchInputValues?: LaunchInputFormValues
        executionLaunchError?: string | null
        executionLastLaunchFailure?: LaunchFailureDiagnostics | null
        executionRunStartGitPolicyWarning?: string | null
        executionCollapsedLaunchInputsByFlow?: Record<string, boolean>
        executionGraphCollapsed?: boolean
        executionOpenRunsAfterLaunch?: boolean
        executionLaunchSuccessRunId?: string | null
    }) => void
}
