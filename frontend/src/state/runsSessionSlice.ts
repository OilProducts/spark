import { type StateCreator } from 'zustand'
import { RUN_METADATA_STALE_AFTER_MS } from '@/lib/runMetadataFreshness'
import type { AppState } from './store-types'
import type {
    RunDetailSessionState,
    RunsListSessionState,
    RunsSessionSlice,
} from './viewSessionTypes'

const resolveMetadataStaleAfterMs = () => {
    const override = (globalThis as typeof globalThis & { __RUNS_METADATA_STALE_AFTER_MS__?: unknown })
        .__RUNS_METADATA_STALE_AFTER_MS__
    return typeof override === 'number' && Number.isFinite(override) && override > 0
        ? override
        : RUN_METADATA_STALE_AFTER_MS
}

const DEFAULT_RUNS_LIST_SESSION_STATE: RunsListSessionState = {
    scopeMode: 'active',
    selectedRunIdByScopeKey: {},
    status: 'idle',
    error: null,
    runs: [],
    lastFetchedAtMs: null,
    nowMs: Date.now(),
    metadataStaleAfterMs: resolveMetadataStaleAfterMs(),
}

const DEFAULT_RUN_DETAIL_SESSION_STATE: RunDetailSessionState = {
    summaryRecord: null,
    completedNodesSnapshot: [],
    statusFetchedAtMs: null,
    graphStatus: 'idle',
    graphError: null,
    graphNodes: [],
    graphEdges: [],
    graphLastLayoutMs: 0,
    checkpointData: null,
    checkpointStatus: 'idle',
    checkpointError: null,
    isSummaryCollapsed: false,
    isActivityCollapsed: false,
    isRawLogsCollapsed: true,
    isTimelineCollapsed: false,
    isCheckpointCollapsed: false,
    isContextCollapsed: false,
    isArtifactsCollapsed: false,
    isGraphCollapsed: true,
    contextData: null,
    contextStatus: 'idle',
    contextError: null,
    contextSearchQuery: '',
    contextCopyStatus: '',
    artifactData: null,
    artifactStatus: 'idle',
    artifactError: null,
    selectedArtifactPath: null,
    artifactViewerStatus: 'idle',
    artifactViewerPayload: '',
    artifactViewerError: null,
    questionsStatus: 'idle',
    pendingQuestionSnapshots: [],
    timelineEvents: [],
    timelineError: null,
    isTimelineLive: false,
    timelineSequence: 0,
    timelineTypeFilter: 'all',
    timelineNodeStageFilter: '',
    timelineCategoryFilter: 'all',
    timelineSeverityFilter: 'all',
    pendingGateActionError: null,
    submittingGateIds: {},
    answeredGateIds: {},
    freeformAnswersByGateId: {},
}

const resolveRunDetailSession = (
    sessionsByRunId: Record<string, RunDetailSessionState>,
    runId: string,
) => ({
    ...DEFAULT_RUN_DETAIL_SESSION_STATE,
    ...(sessionsByRunId[runId] ?? {}),
})

export const createRunsSessionSlice: StateCreator<AppState, [], [], RunsSessionSlice> = (set) => ({
    runsListSession: DEFAULT_RUNS_LIST_SESSION_STATE,
    runDetailSessionsByRunId: {},
    updateRunsListSession: (patch) =>
        set((state) => ({
            runsListSession: {
                ...state.runsListSession,
                ...patch,
            },
        })),
    setRunsSelectedRunIdForScope: (scopeKey, runId) =>
        set((state) => ({
            runsListSession: {
                ...state.runsListSession,
                selectedRunIdByScopeKey: {
                    ...state.runsListSession.selectedRunIdByScopeKey,
                    [scopeKey]: runId,
                },
            },
        })),
    updateRunDetailSession: (runId, patch) =>
        set((state) => ({
            runDetailSessionsByRunId: {
                ...state.runDetailSessionsByRunId,
                [runId]: {
                    ...resolveRunDetailSession(state.runDetailSessionsByRunId, runId),
                    ...patch,
                },
            },
        })),
    clearRunDetailSession: (runId) =>
        set((state) => {
            const next = { ...state.runDetailSessionsByRunId }
            delete next[runId]
            return {
                runDetailSessionsByRunId: next,
            }
        }),
    pruneRunDetailSessions: (runIds) =>
        set((state) => {
            const keepIds = new Set(runIds)
            return {
                runDetailSessionsByRunId: Object.fromEntries(
                    Object.entries(state.runDetailSessionsByRunId).filter(([runId]) => keepIds.has(runId)),
                ),
            }
        }),
})
