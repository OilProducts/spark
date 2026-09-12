import { unconfirmRunQuestions, getRunsSelectedRunIdForScope } from './runsSessionScope'
import { type StateCreator } from 'zustand'
import type { AppState } from './store-types'
import type {
    RunDetailSessionState,
    RunsListSessionState,
    RunsSessionSlice,
} from './viewSessionTypes'

const DEFAULT_RUNS_LIST_SESSION_STATE: RunsListSessionState = {
    scopeMode: 'active',
    selectedRunIdByScopeKey: {},
    status: 'idle',
    error: null,
    runs: [],
    streamStatus: 'idle',
    streamError: null,
}

const DEFAULT_RUN_DETAIL_SESSION_STATE: RunDetailSessionState = {
    lifetime: 0,
    resourceRequestIds: {},
    nodeStatuses: {},
    humanGate: null,
    graphAttrs: {},
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    graphExpanded: null,
    record: null,
    recordUpdates: {},
    statusSync: 'idle',
    statusError: null,
    completedNodesSnapshot: [],
    statusFetchedAtMs: null,
    selectedNodeId: null,
    activityMode: null,
    inspectorTab: null,
    graphStatus: 'idle',
    graphError: null,
    expandChildFlows: false,
    graphNodes: [],
    graphEdges: [],
    graphLastLayoutMs: 0,
    graphPaneHeight: 512,
    checkpointData: null,
    checkpointStatus: 'idle',
    checkpointError: null,
    resultData: null,
    resultStatus: 'idle',
    resultError: null,
    contextData: null,
    contextStatus: 'idle',
    contextError: null,
    contextSearchQuery: '',
    contextCopyStatus: '',
    artifactData: null,
    artifactStatus: 'idle',
    artifactError: null,
    selectedArtifactPath: null,
    artifactViewerRequestId: null,
    artifactViewerStatus: 'idle',
    artifactViewerPayload: '',
    artifactViewerError: null,
    questionsStatus: 'idle',
    pendingQuestionSnapshots: [],
    timelineCategoryFilter: 'all',
    timelineSeverityFilter: 'all',
    pendingGateActionError: null,
    submittingGateIds: {},
    answeredGateIds: {},
    freeformAnswersByGateId: {},
    gateNotesByGateId: {},
}

let nextSessionLifetime = 0

const resolveRunDetailSession = (
    sessionsByRunId: Record<string, RunDetailSessionState>,
    runId: string,
    preferences: AppState['clientRunPresentation'] = {},
) => sessionsByRunId[runId] ?? ({
    ...DEFAULT_RUN_DETAIL_SESSION_STATE,
    activityMode: preferences.activity_mode ?? null,
    inspectorTab: preferences.inspector_tab ?? null,
    timelineCategoryFilter: preferences.timeline_category ?? 'all',
    timelineSeverityFilter: preferences.timeline_severity ?? 'all',
    graphPaneHeight: preferences.graph_height ?? 512,
    lifetime: ++nextSessionLifetime,
})

const LIVE_FIELDS = ['status', 'outcome', 'outcome_reason_code', 'outcome_reason_message', 'ended_at', 'last_error', 'token_usage', 'token_usage_breakdown', 'estimated_model_cost'] as const
const TELEMETRY_FIELDS = ['token_usage', 'token_usage_breakdown', 'estimated_model_cost'] as const

// Track every intended event/action write, even when it matches a list summary.
// Entry identity lets a status request recognize intervening writes to each field.
const trackRecordUpdates = (session: RunDetailSessionState, patch: Partial<NonNullable<RunDetailSessionState['record']>>) => ({
    ...session.recordUpdates,
    ...Object.fromEntries(Object.entries(patch)
        .map(([key, value]) => [key, { value }])),
})

export const createRunsSessionSlice: StateCreator<AppState, [], [], RunsSessionSlice> = (set, get) => ({
    runsListSession: DEFAULT_RUNS_LIST_SESSION_STATE,
    runDetailSessionsByRunId: {},
    reconcileRunRecord: (runId, source, record, completedNodes = [], requestUpdates) => set((state) => {
        const session = state.runDetailSessionsByRunId[runId]
        if (!session) return state
        const current = session.record
        let next = current
        let recordUpdates = session.recordUpdates
        if (source === 'status') {
            const newerFields = requestUpdates === undefined ? {} : Object.fromEntries(
                Object.entries(recordUpdates)
                    .filter(([key, update]) => update !== requestUpdates[key as keyof typeof requestUpdates])
                    .map(([key, update]) => [key, update.value]),
            )
            next = { ...record, ...newerFields } as NonNullable<typeof current>
            recordUpdates = trackRecordUpdates(session, Object.fromEntries(
                Object.entries(record).filter(([key]) => !(key in newerFields)),
            ))
        } else if (source === 'list' && session.statusFetchedAtMs === null) {
            next = record as NonNullable<typeof current>
        } else {
            // Unselected runs have no detail stream; refreshed summaries must
            // update their cached status as well as their telemetry.
            const refreshInactiveRun = source === 'list'
                && runId !== getRunsSelectedRunIdForScope(state.runsListSession, state.activeProjectPath)
            const fields = refreshInactiveRun ? [...LIVE_FIELDS, 'current_node'] as const
                : source === 'live' ? LIVE_FIELDS : TELEMETRY_FIELDS
            const patch = source === 'journal' ? record : Object.fromEntries(
                fields
                    .filter((key) => source === 'live' || refreshInactiveRun ? record[key] !== undefined : record[key] != null)
                    .map((key) => [key, record[key]]),
            )
            if (source === 'live' || source === 'journal' || refreshInactiveRun) recordUpdates = trackRecordUpdates(session, patch)
            if (current) next = { ...current, ...patch }
        }
        return { runDetailSessionsByRunId: { ...state.runDetailSessionsByRunId, [runId]: {
            ...session, record: next, recordUpdates,
            ...(source === 'status' ? { completedNodesSnapshot: completedNodes, statusFetchedAtMs: Date.now(), statusSync: 'ready' as const, statusError: null } : {}),
        } } }
    }),
    optimisticallyPatchRun: (runId, patch) => {
        const before = get()
        const oldSession = before.runDetailSessionsByRunId[runId]
        const oldSummary = before.runsListSession.runs.find((run) => run.run_id === runId)
        let optimisticUpdates: RunDetailSessionState['recordUpdates'] = {}
        let optimisticSummary = oldSummary
        const apply = (rollback: boolean) => set((state) => {
            const appliedPatch = (record: import('@/features/runs/model/shared').RunRecord, previous: typeof record | null | undefined) => {
                if (!rollback) return patch
                if (!previous) return {}
                return Object.fromEntries(
                    Object.entries(patch).filter(([key, value]) => record[key as keyof typeof record] === value)
                        .map(([key]) => [key, previous[key as keyof typeof previous]]),
                )
            }
            const session = state.runDetailSessionsByRunId[runId]
            if (oldSession && session?.lifetime !== oldSession.lifetime) return state
            const recordPatch = session?.record ? Object.fromEntries(
                Object.entries(appliedPatch(session.record, oldSession?.record ?? oldSummary))
                    .filter(([key]) => !rollback || session.recordUpdates[key as keyof typeof optimisticUpdates] === optimisticUpdates[key as keyof typeof optimisticUpdates]),
            ) : {}
            const nextRecord = session?.record ? { ...session.record, ...recordPatch } : null
            return {
                runsListSession: { ...state.runsListSession, runs: state.runsListSession.runs.map((run) => run.run_id === runId && (!rollback || run === optimisticSummary) ? { ...run, ...appliedPatch(run, oldSummary) } : run) },
                ...(session ? { runDetailSessionsByRunId: { ...state.runDetailSessionsByRunId, [runId]: {
                    ...session, record: nextRecord,
                    recordUpdates: nextRecord ? trackRecordUpdates(session, recordPatch) : session.recordUpdates,
                } } } : {}),
            }
        })
        apply(false)
        optimisticUpdates = get().runDetailSessionsByRunId[runId]?.recordUpdates ?? {}
        optimisticSummary = get().runsListSession.runs.find((run) => run.run_id === runId)
        return () => apply(true)
    },
    updateRunsListSession: (patch, source = 'list') => {
        set((state) => {
            const runsListSession = { ...state.runsListSession, ...patch }
            const runId = getRunsSelectedRunIdForScope(runsListSession, state.activeProjectPath)
            const session = runId ? state.runDetailSessionsByRunId[runId] : null
            return { runsListSession, ...(runId && session && runId !== getRunsSelectedRunIdForScope(state.runsListSession, state.activeProjectPath)
                ? { runDetailSessionsByRunId: { ...state.runDetailSessionsByRunId, [runId]: unconfirmRunQuestions(session) } } : {}) }
        })
        if (source === 'list') {
            patch.runs?.forEach((record) => get().reconcileRunRecord(record.run_id, 'list', record))
        }
    },
    setRunsSelectedRunIdForScope: (scopeKey, runId) =>
        set((state) => state.runsListSession.selectedRunIdByScopeKey[scopeKey] === runId ? state : ({
            ...(runId ? { runDetailSessionsByRunId: {
                ...state.runDetailSessionsByRunId,
                [runId]: { ...unconfirmRunQuestions(resolveRunDetailSession(state.runDetailSessionsByRunId, runId, state.clientRunPresentation)),
                    record: state.runDetailSessionsByRunId[runId]?.record ?? state.runsListSession.runs.find((run) => run.run_id === runId) ?? null,
                    questionsStatus: 'idle' as const,
                },
            } } : {}),
            runsListSession: {
                ...state.runsListSession,
                selectedRunIdByScopeKey: {
                    ...state.runsListSession.selectedRunIdByScopeKey,
                    [scopeKey]: runId,
                },
            },
        })),
    updateRunDetailSession: (runId, patch) =>
        set((state) => !state.runDetailSessionsByRunId[runId] ? state : ({
            runDetailSessionsByRunId: {
                ...state.runDetailSessionsByRunId,
                [runId]: {
                    ...resolveRunDetailSession(state.runDetailSessionsByRunId, runId, state.clientRunPresentation),
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
                runsListSession: { ...state.runsListSession, selectedRunIdByScopeKey: Object.fromEntries(
                    Object.entries(state.runsListSession.selectedRunIdByScopeKey).map(([key, id]) => [key, id === runId ? null : id]),
                ) },
            }
        }),
})
