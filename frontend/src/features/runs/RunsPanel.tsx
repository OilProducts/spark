import { useCallback, useEffect } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useRunsList } from './hooks/useRunsList'
import { useRunActions } from './hooks/useRunActions'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { RunActivityCard } from './components/RunActivityCard'
import { RunArtifactsCard } from './components/RunArtifactsCard'
import { RunCheckpointCard } from './components/RunCheckpointCard'
import { RunContextCard } from './components/RunContextCard'
import { RunEventTimelineCard } from './components/RunEventTimelineCard'
import { RunGraphCard } from './components/RunGraphCard'
import { RunList } from './components/RunList'
import { RunSummaryCard } from './components/RunSummaryCard'
import type { RunRecord } from './model/shared'
import type { RunDetailSessionState } from '@/state/viewSessionTypes'
import { buildRunsScopeKey, getRunsSelectedRunIdForScope } from '@/state/runsSessionScope'
import { InlineNotice } from '@/ui'

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
        'current_node',
        'continued_from_run_id',
        'continued_from_node',
        'continued_from_flow_mode',
        'continued_from_flow_name',
    ].every((key) => left[key as keyof RunRecord] === right[key as keyof RunRecord])
}

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
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setSelectedRunSnapshot = useStore((state) => state.setSelectedRunSnapshot)
    const setViewMode = useStore((state) => state.setViewMode)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setExecutionContinuation = useStore((state) => state.setExecutionContinuation)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const setModel = useStore((state) => state.setModel)
    const setSpecId = useStore((state) => state.setSpecId)
    const setPlanId = useStore((state) => state.setPlanId)
    const {
        error,
        fetchRuns,
        isLoading,
        metadataFreshness,
        now,
        scopedRuns,
        selectedRunSummary,
        setRuns,
        status,
        summary,
    } = useRunsList({
        activeProjectPath,
        scopeMode,
        selectedRunId,
        manageSync: false,
    })
    const { requestCancel } = useRunActions({
        fetchRuns,
        setRuns,
    })
    const selectedRunDetailSession = useStore((state) => (
        selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? null : null
    ))
    const hasScopedSelectedRun = selectedRunId
        ? scopedRuns.some((run) => run.run_id === selectedRunId)
        : false
    const selectedRunSessionRecord = selectedRunDetailSession?.summaryRecord ?? null
    const selectedRun =
        selectedRunSummary
            ?? (
                selectedRunSessionRecord
                && selectedRunSessionRecord.run_id === selectedRunId
                && (isLoading || Boolean(error) || hasScopedSelectedRun || scopedRuns.length === 0)
                    ? selectedRunSessionRecord
                    : null
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
        filteredContextRows,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
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
    const {
        filteredTimelineEvents,
        freeformAnswersByGateId,
        groupedPendingInterviewGates,
        groupedTimelineEntries,
        isTimelineLive,
        pendingGateActionError,
        setFreeformAnswersByGateId,
        setTimelineCategoryFilter,
        setTimelineNodeStageFilter,
        setTimelineSeverityFilter,
        setTimelineTypeFilter,
        submittingGateIds,
        submitPendingGateAnswer,
        timelineCategoryFilter,
        timelineDroppedCount,
        timelineError,
        timelineEvents,
        timelineNodeStageFilter,
        timelineSeverityFilter,
        timelineTypeFilter,
        timelineTypeOptions,
        visiblePendingInterviewGates,
    } = useRunTimeline({
        pendingQuestionSnapshots,
        selectedRunTimelineId,
        manageSync: false,
    })
    const selectedRunSessionState = useStore((state) => (
        selectedRun?.run_id ? state.runDetailSessionsByRunId[selectedRun.run_id] ?? null : null
    ))
    const isSummaryCollapsed = selectedRunSessionState?.isSummaryCollapsed ?? false
    const isActivityCollapsed = selectedRunSessionState?.isActivityCollapsed ?? false
    const isRawLogsCollapsed = selectedRunSessionState?.isRawLogsCollapsed ?? true
    const isTimelineCollapsed = selectedRunSessionState?.isTimelineCollapsed ?? false
    const isCheckpointCollapsed = selectedRunSessionState?.isCheckpointCollapsed ?? false
    const isContextCollapsed = selectedRunSessionState?.isContextCollapsed ?? false
    const isArtifactsCollapsed = selectedRunSessionState?.isArtifactsCollapsed ?? false
    const patchSelectedRunSession = useCallback((patch: Partial<RunDetailSessionState>) => {
        if (!selectedRun?.run_id) {
            return
        }
        updateRunDetailSession(selectedRun.run_id, patch)
    }, [selectedRun?.run_id, updateRunDetailSession])
    const degradedRunPanels = timelineError
        ? [...degradedDetailPanels, 'event timeline']
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

    const openRunArtifact = (run: RunRecord, artifactType: 'spec' | 'plan') => {
        const artifactId = artifactType === 'spec' ? run.spec_id : run.plan_id
        if (!artifactId) {
            return
        }
        if (run.project_path) {
            setActiveProjectPath(run.project_path)
        }
        if (artifactType === 'spec') {
            setSpecId(artifactId)
        } else {
            setPlanId(artifactId)
        }
        setViewMode('home')
    }

    return (
        <section
            data-testid="runs-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`flex-1 ${isNarrowViewport ? 'overflow-auto p-3' : 'flex min-h-0 flex-col overflow-hidden p-6'}`}
        >
            <div className={`w-full ${isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 overflow-hidden'}`}>
                <RunList
                    activeProjectPath={activeProjectPath}
                    error={error}
                    isLoading={isLoading}
                    metadataFreshness={metadataFreshness}
                    onRefresh={() => {
                        void fetchRuns()
                    }}
                    scopeMode={scopeMode}
                    onScopeModeChange={(mode) => {
                        updateRunsListSession({ scopeMode: mode })
                    }}
                    status={status}
                    now={now}
                    onSelectRun={selectRun}
                    runs={scopedRuns}
                    selectedRunId={selectedRunId}
                    summaryLabel={`${summary.total} total runs · ${summary.running} running`}
                />
                <div className={`min-w-0 ${isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 flex-col overflow-hidden pl-6'}`}>
                    <div className={isNarrowViewport ? 'space-y-6' : 'min-h-0 flex-1 space-y-6 overflow-auto pr-2'}>
                        {showRunSelectionEmptyState && (
                            <div data-testid="run-selection-empty-state" className="rounded-md border border-border bg-card px-3 py-2 text-sm text-muted-foreground">
                                Select a run from the sidebar to inspect its details.
                            </div>
                        )}
                        {showRunDetailsRestoringState && (
                            <InlineNotice data-testid="run-selection-restoring-state">
                                Restoring the selected run session…
                            </InlineNotice>
                        )}
                        {selectedRun && (
                            <RunSummaryCard
                                activeProjectPath={activeProjectPath}
                                collapsed={isSummaryCollapsed}
                                now={now}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isSummaryCollapsed: collapsed })
                                }}
                                onContinueFromRun={beginContinuation}
                                onOpenRunArtifact={openRunArtifact}
                                onRequestCancel={(runId, currentStatus) => {
                                    void requestCancel(runId, currentStatus)
                                }}
                                run={selectedRun}
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
                            <InlineNotice>
                                No runs have been recorded yet.
                            </InlineNotice>
                        )}
                        {selectedRun && (
                            <RunActivityCard
                                key={`activity-${selectedRun.run_id}`}
                                checkpointCompletedNodes={checkpointCompletedNodes}
                                checkpointCurrentNode={checkpointCurrentNode}
                                checkpointRetryCounters={checkpointRetryCounters}
                                collapsed={isActivityCollapsed}
                                groupedTimelineEntries={groupedTimelineEntries}
                                pendingGateCount={visiblePendingInterviewGates.length}
                                rawLogsCollapsed={isRawLogsCollapsed}
                                run={selectedRun}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isActivityCollapsed: collapsed })
                                }}
                                onRawLogsCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isRawLogsCollapsed: collapsed })
                                }}
                            />
                        )}
                        {selectedRun && (
                            <RunEventTimelineCard
                                collapsed={isTimelineCollapsed}
                                isNarrowViewport={isNarrowViewport}
                                isTimelineLive={isTimelineLive}
                                timelineDroppedCount={timelineDroppedCount}
                                timelineError={timelineError}
                                timelineEvents={timelineEvents}
                                visiblePendingInterviewGates={visiblePendingInterviewGates}
                                groupedPendingInterviewGates={groupedPendingInterviewGates}
                                pendingGateActionError={pendingGateActionError}
                                submittingGateIds={submittingGateIds}
                                freeformAnswersByGateId={freeformAnswersByGateId}
                                timelineTypeFilter={timelineTypeFilter}
                                timelineTypeOptions={timelineTypeOptions}
                                timelineNodeStageFilter={timelineNodeStageFilter}
                                timelineCategoryFilter={timelineCategoryFilter}
                                timelineSeverityFilter={timelineSeverityFilter}
                                filteredTimelineEvents={filteredTimelineEvents}
                                groupedTimelineEntries={groupedTimelineEntries}
                                onTimelineCategoryFilterChange={setTimelineCategoryFilter}
                                onTimelineNodeStageFilterChange={setTimelineNodeStageFilter}
                                onTimelineSeverityFilterChange={setTimelineSeverityFilter}
                                onTimelineTypeFilterChange={setTimelineTypeFilter}
                                onFreeformAnswerChange={(questionId, value) => {
                                    setFreeformAnswersByGateId((previous) => ({
                                        ...previous,
                                        [questionId]: value,
                                    }))
                                }}
                                onSubmitPendingGateAnswer={(gate, selectedValue) => {
                                    void submitPendingGateAnswer(gate, selectedValue)
                                }}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isTimelineCollapsed: collapsed })
                                }}
                            />
                        )}
                        {selectedRun && (
                            <RunCheckpointCard
                                collapsed={isCheckpointCollapsed}
                                checkpointCompletedNodes={checkpointCompletedNodes}
                                checkpointCurrentNode={checkpointCurrentNode}
                                checkpointData={checkpointData?.checkpoint ?? null}
                                checkpointError={checkpointError}
                                checkpointRetryCounters={checkpointRetryCounters}
                                isLoading={isCheckpointLoading}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isCheckpointCollapsed: collapsed })
                                }}
                                onRefresh={() => {
                                    void fetchCheckpoint()
                                }}
                                status={checkpointStatus}
                            />
                        )}
                        {selectedRun && (
                            <RunGraphCard
                                key={`graph-${selectedRun.run_id}`}
                                run={selectedRun}
                            />
                        )}
                        {selectedRun && (
                            <RunContextCard
                                collapsed={isContextCollapsed}
                                contextCopyStatus={contextCopyStatus}
                                contextError={contextError}
                                contextExportHref={contextExportHref || null}
                                filteredContextRows={filteredContextRows}
                                isLoading={isContextLoading}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isContextCollapsed: collapsed })
                                }}
                                onCopy={() => {
                                    void copyContextToClipboard()
                                }}
                                onRefresh={() => {
                                    setContextCopyStatus('')
                                    void fetchContext()
                                }}
                                onSearchQueryChange={setContextSearchQuery}
                                runId={selectedRun.run_id}
                                searchQuery={contextSearchQuery}
                                status={contextStatus}
                            />
                        )}
                        {selectedRun && (
                            <RunArtifactsCard
                                artifactDownloadHref={(artifactPath) => artifactDownloadHref(artifactPath) || null}
                                artifactEntries={artifactEntries}
                                artifactError={artifactError}
                                artifactViewerError={artifactViewerError}
                                artifactViewerPayload={artifactViewerPayload || null}
                                collapsed={isArtifactsCollapsed}
                                isArtifactViewerLoading={isArtifactViewerLoading}
                                isLoading={isArtifactLoading}
                                missingCoreArtifacts={missingCoreArtifacts}
                                onCollapsedChange={(collapsed) => {
                                    patchSelectedRunSession({ isArtifactsCollapsed: collapsed })
                                }}
                                onRefresh={() => {
                                    void fetchArtifacts()
                                }}
                                onViewArtifact={(artifact) => {
                                    void viewArtifact(artifact)
                                }}
                                selectedArtifactEntry={selectedArtifactEntry}
                                showPartialRunArtifactNote={showPartialRunArtifactNote}
                                status={artifactStatus}
                            />
                        )}
                    </div>
                </div>
            </div>
        </section>
    )
}
