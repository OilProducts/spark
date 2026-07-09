import { useCallback, useEffect, useMemo, useRef } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useRunsList } from './hooks/useRunsList'
import { useRunActions } from './hooks/useRunActions'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { useRunTranscriptStore } from './state/runTranscriptStore'
import { buildRunTranscriptGroups } from './model/transcriptModel'
import { RunActivityCard } from './components/RunActivityCard'
import { RunGraphCard } from './components/RunGraphCard'
import { RunInspectorPanel } from './components/RunInspectorPanel'
import { RunList } from './components/RunList'
import { RunHeaderBar } from './components/RunHeaderBar'
import { RunQuestionsPanel } from './components/RunQuestionsPanel'
import { type RunRecord } from './model/shared'
import { buildRunNodeStatuses } from './model/nodeStatusModel'
import { nodeOutcomesFromCheckpoint } from './model/runDetailsModel'
import type { RunDetailSessionState } from '@/state/viewSessionTypes'
import { buildRunsScopeKey, getRunsSelectedRunIdForScope } from '@/state/runsSessionScope'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { requestRunsTransportReconnect } from './services/runsTransportReconnect'
import type { RunTranscriptSegment } from '@/lib/api/attractorApi'

const EMPTY_TRANSCRIPT_SEGMENTS: RunTranscriptSegment[] = []

const runRecordsMatch = (left: RunRecord | null, right: RunRecord | null) => {
    if (left === right) {
        return true
    }
    if (!left || !right) {
        return false
    }
    return [
        'run_id',
        'flow_name',
        'status',
        'outcome',
        'outcome_reason_code',
        'outcome_reason_message',
        'working_directory',
        'project_path',
        'git_branch',
        'git_commit',
        'spec_id',
        'plan_id',
        'model',
        'started_at',
        'ended_at',
        'last_error',
        'token_usage',
        'token_usage_breakdown',
        'estimated_model_cost',
        'current_node',
        'continued_from_run_id',
        'continued_from_node',
        'continued_from_flow_mode',
        'continued_from_flow_name',
        'parent_run_id',
        'parent_node_id',
        'root_run_id',
        'child_invocation_index',
        'execution_lock',
    ].every((key) => {
        const leftValue = left[key as keyof RunRecord]
        const rightValue = right[key as keyof RunRecord]
        if (key === 'token_usage_breakdown' || key === 'estimated_model_cost' || key === 'execution_lock') {
            return JSON.stringify(leftValue ?? null) === JSON.stringify(rightValue ?? null)
        }
        return leftValue === rightValue
    })
}

const mergeSelectedRunTelemetry = (currentRecord: RunRecord, summaryRecord: RunRecord): RunRecord => ({
    ...currentRecord,
    token_usage: summaryRecord.token_usage ?? currentRecord.token_usage,
    token_usage_breakdown: summaryRecord.token_usage_breakdown ?? currentRecord.token_usage_breakdown,
    estimated_model_cost: summaryRecord.estimated_model_cost ?? currentRecord.estimated_model_cost,
})

const ACTIVE_RUN_STATUSES = new Set(['running', 'pause_requested', 'abort_requested', 'cancel_requested'])


export function RunsPanel() {
    const isNarrowViewport = useNarrowViewport()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const runsListSession = useStore((state) => state.runsListSession)
    const scopeMode = runsListSession.scopeMode
    const updateRunsListSession = useStore((state) => state.updateRunsListSession)
    const setRunsSelectedRunIdForScope = useStore((state) => state.setRunsSelectedRunIdForScope)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const globalSelectedRunId = useStore((state) => state.selectedRunId)
    const selectedRunId = getRunsSelectedRunIdForScope(runsListSession, activeProjectPath) ?? globalSelectedRunId
    const selectedRunRecord = useStore((state) => state.selectedRunRecord)
    const selectedRunStatusFetchedAtMs = useStore((state) => state.selectedRunStatusFetchedAtMs)
    const selectedRunStatusSync = useStore((state) => state.selectedRunStatusSync)
    const selectedRunStatusError = useStore((state) => state.selectedRunStatusError)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setSelectedRunSnapshot = useStore((state) => state.setSelectedRunSnapshot)
    const setViewMode = useStore((state) => state.setViewMode)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setExecutionContinuation = useStore((state) => state.setExecutionContinuation)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const setModel = useStore((state) => state.setModel)
    const {
        error,
        isLoading,
        scopedRuns,
        selectedRunSummary,
        setRuns,
        status,
        streamError,
        streamStatus,
        summary,
    } = useRunsList({
        activeProjectPath,
        scopeMode,
        selectedRunId,
        manageSync: false,
    })
    const { requestCancel, requestRetry } = useRunActions({ setRuns })
    const selectedRunDetailSession = useStore((state) => (
        selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? null : null
    ))
    const hasScopedSelectedRun = selectedRunId
        ? scopedRuns.some((run) => run.run_id === selectedRunId)
        : false
    const authoritativeSelectedRunRecord = selectedRunRecord?.run_id === selectedRunId
        ? selectedRunRecord
        : null
    const selectedRunSessionRecord = selectedRunDetailSession?.summaryRecord ?? null
    const selectedRun =
        authoritativeSelectedRunRecord
        ?? (
            selectedRunSessionRecord
            && selectedRunSessionRecord.run_id === selectedRunId
                ? selectedRunSessionRecord
                : (
                    selectedRunSummary
                    ?? (
                        selectedRunSessionRecord
                        && selectedRunSessionRecord.run_id === selectedRunId
                        && (isLoading || Boolean(error) || hasScopedSelectedRun || scopedRuns.length === 0)
                            ? selectedRunSessionRecord
                            : null
                    )
                )
        )
    const selectedRunTimelineId = selectedRun?.run_id ?? null
    const {
        artifactDownloadHref,
        artifactEntries,
        artifactError,
        artifactStatus,
        artifactViewerError,
        artifactViewerPayload,
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointData,
        checkpointError,
        checkpointStatus,
        checkpointRetryCounters,
        contextCopyStatus,
        contextError,
        contextExportHref,
        contextSearchQuery,
        contextStatus,
        degradedDetailPanels,
        fetchArtifacts,
        fetchCheckpoint,
        fetchContext,
        fetchResult,
        filteredContextRows,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        isResultLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
        resultData,
        resultError,
        resultStatus,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
        showPartialRunArtifactNote,
        viewArtifact,
        copyContextToClipboard,
    } = useRunDetails({
        selectedRunSummary: selectedRun,
        manageSync: false,
    })
    const checkpointResumeNode = checkpointCurrentNode !== '—' ? checkpointCurrentNode : null
    const transcriptState = useRunTranscriptStore((state) => (
        selectedRun ? state.byRunId[selectedRun.run_id] : undefined
    ))
    const transcriptSegments = transcriptState?.segments ?? EMPTY_TRANSCRIPT_SEGMENTS
    const transcriptError = transcriptState?.status === 'error' ? transcriptState.error : null
    const {
        filteredTimelineEventCount,
        freeformAnswersByGateId,
        groupedPendingInterviewGates,
        groupedTimelineEntries,
        hasOlderTimelineEvents,
        isTimelineLive,
        isTimelineLoadingOlder,
        loadOlderTimelineEvents,
        pendingGateActionError,
        setFreeformAnswersByGateId,
        setTimelineCategoryFilter,
        setTimelineSeverityFilter,
        submittingGateIds,
        submitPendingGateAnswer,
        timelineCategoryFilter,
        timelineError,
        timelineEventCount,
        timelineSeverityFilter,
        visiblePendingInterviewGates,
    } = useRunTimeline({
        pendingQuestionSnapshots,
        selectedRunCurrentNode: selectedRun?.current_node ?? checkpointResumeNode,
        selectedRunTimelineId,
    })
    const selectedRunSessionState = useStore((state) => (
        selectedRun?.run_id ? state.runDetailSessionsByRunId[selectedRun.run_id] ?? null : null
    ))
    const storedActivityMode = selectedRunSessionState?.activityMode ?? null
    // Transcript-first: with no explicit mode chosen, lead with the live
    // transcript whenever the run has agent output to show; tool-only runs
    // fall back to the full activity stream.
    const hasTranscriptContent = useMemo(
        () => buildRunTranscriptGroups(transcriptSegments, null).length > 0,
        [transcriptSegments],
    )
    const activityMode = storedActivityMode ?? (hasTranscriptContent ? 'transcript' : 'all')
    const patchSelectedRunSession = useCallback((patch: Partial<RunDetailSessionState>) => {
        if (!selectedRun?.run_id) {
            return
        }
        updateRunDetailSession(selectedRun.run_id, patch)
    }, [selectedRun?.run_id, updateRunDetailSession])
    const degradedRunPanels = timelineError
        ? [...degradedDetailPanels, 'run journal']
        : degradedDetailPanels
    const showRunSelectionEmptyState =
        status === 'ready'
        && !selectedRunId
        && (((scopeMode === 'active' && activeProjectPath) || scopeMode === 'all'))
        && scopedRuns.length > 0
        && !selectedRun
    const showRunDetailsRestoringState =
        Boolean(selectedRunId)
        && !selectedRun
        && status !== 'ready'
        && status !== 'error'
    const degradedTransportLabels = [
        ...(streamStatus === 'degraded' ? ['run list'] : []),
        ...(selectedRunStatusSync === 'degraded' ? ['selected run'] : []),
    ]
    const showRunsTransportReconnectNotice = degradedTransportLabels.length > 0
    const runsTransportError = [streamError, selectedRunStatusError].filter(Boolean).join(' ')
    // eslint-disable-next-line react-hooks/purity -- intentional render-time clock snapshot for elapsed-time labels; re-renders are driven by stream events
    const now = Date.now()
    const questionsPanelRef = useRef<HTMLDivElement | null>(null)
    const detailsScrollRef = useRef<HTMLDivElement | null>(null)
    const currentNodeForSummary = selectedRun?.current_node || checkpointResumeNode
    const selectedNodeId = selectedRunSessionState?.selectedNodeId ?? null
    const storedInspectorTab = selectedRunSessionState?.inspectorTab ?? null
    // Activity-first: the live transcript stream is the default work surface;
    // an explicit tab choice sticks per run.
    const inspectorTab = storedInspectorTab ?? 'activity'
    const liveNodeStatuses = useStore((state) => state.nodeStatuses)
    const humanGateNodeId = useStore((state) => state.humanGate?.nodeId ?? null)
    const gateNodeId = visiblePendingInterviewGates[0]?.nodeId ?? humanGateNodeId
    const isSelectedRunActive = selectedRun ? ACTIVE_RUN_STATUSES.has(selectedRun.status) : false
    const completedNodesSnapshot = selectedRunSessionState?.completedNodesSnapshot
    const checkpointNodeOutcomes = useMemo(
        () => nodeOutcomesFromCheckpoint(checkpointData),
        [checkpointData],
    )
    const selectedRunStatus = selectedRun?.status ?? null
    const runNodeStatuses = useMemo(() => buildRunNodeStatuses({
        completedNodes: completedNodesSnapshot ?? [],
        nodeOutcomes: checkpointNodeOutcomes,
        currentNodeId: currentNodeForSummary,
        liveNodeStatuses,
        gateNodeId,
        isRunActive: isSelectedRunActive,
        runStatus: selectedRunStatus,
    }), [
        checkpointNodeOutcomes,
        completedNodesSnapshot,
        currentNodeForSummary,
        gateNodeId,
        isSelectedRunActive,
        liveNodeStatuses,
        selectedRunStatus,
    ])
    // Explicit node selection also focuses the inspector's node tab; the gate
    // auto-focus below deliberately does not, so it never hijacks a tab the
    // operator chose.
    // A new run starts reading from the top; scroll position must not leak
    // from the previous selection.
    useEffect(() => {
        if (detailsScrollRef.current) {
            detailsScrollRef.current.scrollTop = 0
        }
    }, [selectedRun?.run_id])

    const selectNode = useCallback((nodeId: string | null) => {
        if (!selectedRun?.run_id) {
            return
        }
        const patch: Partial<RunDetailSessionState> = { selectedNodeId: nodeId }
        if (nodeId) {
            // Selecting a node is an intent to read its activity: land in the
            // node-scoped live transcript stream.
            patch.inspectorTab = 'activity'
        }
        updateRunDetailSession(selectedRun.run_id, patch)
    }, [selectedRun?.run_id, updateRunDetailSession])

    useEffect(() => {
        if (!selectedNodeId) {
            return
        }
        const onKeyDown = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                selectNode(null)
            }
        }
        window.addEventListener('keydown', onKeyDown)
        return () => {
            window.removeEventListener('keydown', onKeyDown)
        }
    }, [selectedNodeId, selectNode])

    // Focus the gate's node when a run starts waiting on input. Keyed on the gate
    // node so an explicit clear afterwards is respected until the gate set changes.
    useEffect(() => {
        if (!gateNodeId || !selectedRun?.run_id) {
            return
        }
        const session = useStore.getState().runDetailSessionsByRunId[selectedRun.run_id]
        if ((session?.selectedNodeId ?? null) === null) {
            updateRunDetailSession(selectedRun.run_id, { selectedNodeId: gateNodeId })
        }
    }, [gateNodeId, selectedRun?.run_id, updateRunDetailSession])

    const selectRun = (run: RunRecord) => {
        setRunsSelectedRunIdForScope(
            buildRunsScopeKey(scopeMode, activeProjectPath),
            run.run_id,
        )
        setSelectedRunId(run.run_id)
        setSelectedRunSnapshot({ record: run, completedNodes: [] })
    }

    useEffect(() => {
        if (!selectedRunId || !selectedRunSummary) {
            return
        }
        const hasFetchedStatus =
            selectedRunStatusFetchedAtMs !== null
            || (selectedRunSessionState?.statusFetchedAtMs ?? null) !== null
        if (hasFetchedStatus) {
            const currentDetailRecord = selectedRunRecord?.run_id === selectedRunId
                ? selectedRunRecord
                : selectedRunSessionRecord?.run_id === selectedRunId
                    ? selectedRunSessionRecord
                    : null
            if (!currentDetailRecord) {
                return
            }
            const mergedRecord = mergeSelectedRunTelemetry(currentDetailRecord, selectedRunSummary)
            if (runRecordsMatch(currentDetailRecord, mergedRecord)) {
                return
            }
            setSelectedRunSnapshot({
                record: mergedRecord,
                completedNodes: selectedRunSessionState?.completedNodesSnapshot ?? [],
                fetchedAtMs: selectedRunSessionState?.statusFetchedAtMs ?? selectedRunStatusFetchedAtMs,
            })
            return
        }
        if (
            !runRecordsMatch(selectedRunSessionRecord, selectedRunSummary)
            || !runRecordsMatch(selectedRunRecord, selectedRunSummary)
        ) {
            setSelectedRunSnapshot({
                record: selectedRunSummary,
                completedNodes: selectedRunSessionState?.completedNodesSnapshot ?? [],
                fetchedAtMs: selectedRunSessionState?.statusFetchedAtMs ?? null,
            })
        }
    }, [
        selectedRunId,
        selectedRunRecord,
        selectedRunSessionRecord,
        selectedRunSessionState?.completedNodesSnapshot,
        selectedRunSessionState?.statusFetchedAtMs,
        selectedRunStatusFetchedAtMs,
        selectedRunSummary,
        setSelectedRunSnapshot,
    ])

    const beginContinuation = (run: RunRecord) => {
        const projectPath = run.project_path || run.working_directory || null
        const normalizedModel = run.model === 'codex default (config/profile)' ? '' : run.model || ''

        if (projectPath) {
            setActiveProjectPath(projectPath)
        }
        setExecutionFlow(run.flow_name || null)
        setWorkingDir(run.working_directory || projectPath || '')
        setModel(normalizedModel)
        setExecutionContinuation({
            sourceRunId: run.run_id,
            sourceFlowName: run.flow_name || null,
            sourceWorkingDirectory: run.working_directory || projectPath || '',
            sourceModel: run.model || null,
            flowSourceMode: 'snapshot',
            startNodeId: null,
        })
        setViewMode('execution')
    }

    return (
        <section
            data-testid="runs-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`h-full flex-1 ${isNarrowViewport ? 'overflow-auto p-3' : 'flex min-h-0 flex-col overflow-hidden p-6'}`}
        >
            {showRunsTransportReconnectNotice ? (
                <div className="mb-4">
                    <Alert
                        data-testid="runs-transport-reconnect-banner"
                        className="border-amber-500/40 bg-amber-500/10 px-3 py-2 text-amber-800"
                    >
                        <AlertDescription className="text-inherit">
                            Live run transport degraded for {degradedTransportLabels.join(' and ')}.
                            {runsTransportError ? ` ${runsTransportError}` : ''}
                            <button
                                type="button"
                                data-testid="runs-transport-reconnect-button"
                                onClick={() => {
                                    requestRunsTransportReconnect()
                                }}
                                className="ml-2 inline-flex text-xs font-semibold underline underline-offset-4"
                            >
                                Reconnect
                            </button>
                        </AlertDescription>
                    </Alert>
                </div>
            ) : null}
            <div className={`w-full ${isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 overflow-hidden'}`}>
                <RunList
                    activeProjectPath={activeProjectPath}
                    error={error}
                    scopeMode={scopeMode}
                    onScopeModeChange={(mode) => {
                        updateRunsListSession({ scopeMode: mode })
                    }}
                    status={status}
                    onSelectRun={selectRun}
                    runs={scopedRuns}
                    selectedRunId={selectedRunId}
                    summaryLabel={`${summary.total} runs · ${summary.running} running${summary.queued > 0 ? ` · ${summary.queued} queued` : ''}`}
                />
                <div className={`min-w-0 ${isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 flex-col overflow-hidden pl-6'}`}>
                    <div
                        className={isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 flex-col gap-4'}
                    >
                        {showRunSelectionEmptyState && (
                            <div data-testid="run-selection-empty-state" className="rounded-md border border-border bg-card px-3 py-2 text-sm text-muted-foreground">
                                Select a run from the sidebar to inspect its details.
                            </div>
                        )}
                        {showRunDetailsRestoringState && (
                            <Alert
                                data-testid="run-selection-restoring-state"
                                className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                            >
                                <AlertDescription className="text-inherit">
                                    Restoring the selected run session…
                                </AlertDescription>
                            </Alert>
                        )}
                        {selectedRun && (
                            <RunHeaderBar
                                run={selectedRun}
                                now={now}
                                currentNodeId={currentNodeForSummary}
                                onContinueFromRun={beginContinuation}
                                onRequestCancel={(runId, currentStatus) => {
                                    void requestCancel(runId, currentStatus)
                                }}
                                onRequestRetry={(runId, currentStatus) => {
                                    void requestRetry(runId, currentStatus)
                                }}
                                onFocusPendingQuestions={() => {
                                    questionsPanelRef.current?.scrollIntoView({ block: 'start', behavior: 'smooth' })
                                }}
                            />
                        )}
                        {selectedRun && degradedRunPanels.length > 0 && (
                            <div
                                data-testid="run-partial-api-failure-banner"
                                className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-800"
                            >
                                Some run detail endpoints are unavailable. Non-dependent panels remain functional.
                                <span className="ml-1 text-xs">
                                    Affected surfaces: {degradedRunPanels.join(', ')}.
                                </span>
                            </div>
                        )}
                        {!selectedRun && scopeMode === 'all' && scopedRuns.length === 0 && (
                            <Alert className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground">
                                <AlertDescription className="text-inherit">
                                    No runs have been recorded yet.
                                </AlertDescription>
                            </Alert>
                        )}
                        {selectedRun && (
                            <div
                                ref={questionsPanelRef}
                                className={isNarrowViewport ? undefined : 'max-h-[38vh] shrink-0 overflow-y-auto'}
                            >
                            <RunQuestionsPanel
                                freeformAnswersByGateId={freeformAnswersByGateId}
                                groupedPendingInterviewGates={groupedPendingInterviewGates}
                                onFreeformAnswerChange={(questionId, value) => {
                                    setFreeformAnswersByGateId((previous) => ({
                                        ...previous,
                                        [questionId]: value,
                                    }))
                                }}
                                onSubmitPendingGateAnswer={(gate, selectedValue) => {
                                    void submitPendingGateAnswer(gate, selectedValue)
                                }}
                                pendingGateActionError={pendingGateActionError}
                                submittingGateIds={submittingGateIds}
                            />
                            </div>
                        )}
                        {selectedRun && (
                            <div className={isNarrowViewport
                                ? 'space-y-6'
                                : 'flex min-h-0 flex-1 gap-4'}
                            >
                                <div className={isNarrowViewport
                                    ? undefined
                                    : 'flex w-[24rem] min-w-[19rem] shrink-0 flex-col min-h-0 2xl:w-[28rem]'}
                                >
                                    <RunGraphCard
                                        key={`graph-${selectedRun.run_id}`}
                                        run={selectedRun}
                                        nodeStatusesById={runNodeStatuses}
                                        selectedNodeId={selectedNodeId}
                                        onSelectNode={selectNode}
                                        fillHeight={!isNarrowViewport}
                                    />
                                </div>
                                <div className={isNarrowViewport
                                    ? 'mt-6'
                                    : 'flex min-h-0 min-w-0 flex-1 flex-col'}
                                >
                                <RunInspectorPanel
                                    inspectorTab={inspectorTab}
                                    onInspectorTabChange={(tab) => {
                                        patchSelectedRunSession({ inspectorTab: tab })
                                    }}
                                    fillHeight={!isNarrowViewport}
                                    scrollRegionRef={detailsScrollRef}
                                    activityContent={
                                        <RunActivityCard
                                            fillHeight={!isNarrowViewport}
                                        isNarrowViewport={isNarrowViewport}
                                        isLive={isTimelineLive}
                                        activityMode={activityMode}
                                        onActivityModeChange={(mode) => {
                                            patchSelectedRunSession({ activityMode: mode })
                                        }}
                                        selectedNodeId={selectedNodeId}
                                        onClearNodeSelection={() => {
                                            selectNode(null)
                                        }}
                                        transcriptSegments={transcriptSegments}
                                        transcriptError={transcriptError}
                                        groupedTimelineEntries={groupedTimelineEntries}
                                        timelineError={timelineError}
                                        timelineEventCount={timelineEventCount}
                                        filteredTimelineEventCount={filteredTimelineEventCount}
                                        timelineCategoryFilter={timelineCategoryFilter}
                                        timelineSeverityFilter={timelineSeverityFilter}
                                        onTimelineCategoryFilterChange={setTimelineCategoryFilter}
                                        onTimelineSeverityFilterChange={setTimelineSeverityFilter}
                                        hasOlderTimelineEvents={hasOlderTimelineEvents}
                                        isTimelineLoadingOlder={isTimelineLoadingOlder}
                                        onLoadOlderTimelineEvents={() => {
                                            void loadOlderTimelineEvents()
                                        }}
                                        />
                                    }
                                    detailsCardProps={{
                                        run: selectedRun,
                                        activeProjectPath,
                                        now,
                                    }}
                                    resultCardProps={{
                                        result: resultData,
                                        resultError,
                                        isLoading: isResultLoading || resultStatus === 'idle',
                                        onRefresh: () => {
                                            void fetchResult()
                                        },
                                        onViewSource: (artifactPath) => {
                                            void viewArtifact({ path: artifactPath, viewable: true })
                                        },
                                    }}
                                    checkpointCardProps={{
                                        collapsed: false,
                                        checkpointCompletedNodes,
                                        checkpointCurrentNode,
                                        checkpointData: checkpointData?.checkpoint ?? null,
                                        checkpointError,
                                        checkpointRetryCounters,
                                        isLoading: isCheckpointLoading,
                                        onCollapsedChange: () => {},
                                        onRefresh: () => {
                                            void fetchCheckpoint()
                                        },
                                        status: checkpointStatus,
                                    }}
                                    contextCardProps={{
                                        collapsed: false,
                                        contextCopyStatus,
                                        contextError,
                                        contextExportHref: contextExportHref || null,
                                        filteredContextRows,
                                        isLoading: isContextLoading,
                                        onCollapsedChange: () => {},
                                        onCopy: () => {
                                            void copyContextToClipboard()
                                        },
                                        onRefresh: () => {
                                            setContextCopyStatus('')
                                            void fetchContext()
                                        },
                                        onSearchQueryChange: setContextSearchQuery,
                                        runId: selectedRun.run_id,
                                        searchQuery: contextSearchQuery,
                                        status: contextStatus,
                                    }}
                                    artifactsCardProps={{
                                        artifactDownloadHref: (artifactPath) => artifactDownloadHref(artifactPath) || null,
                                        artifactEntries,
                                        artifactError,
                                        artifactViewerError,
                                        artifactViewerPayload: artifactViewerPayload || null,
                                        collapsed: false,
                                        isArtifactViewerLoading,
                                        isLoading: isArtifactLoading,
                                        missingCoreArtifacts,
                                        onCollapsedChange: () => {},
                                        onRefresh: () => {
                                            void fetchArtifacts()
                                        },
                                        onViewArtifact: (artifact) => {
                                            void viewArtifact(artifact)
                                        },
                                        selectedArtifactEntry,
                                        showPartialRunArtifactNote,
                                        status: artifactStatus,
                                    }}
                                />
                                </div>
                            </div>
                        )}
                    </div>
                </div>
            </div>
        </section>
    )
}
