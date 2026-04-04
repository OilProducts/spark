import { useEffect, useState } from 'react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useRunsList } from './hooks/useRunsList'
import { useRunActions } from './hooks/useRunActions'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { RunArtifactsCard } from './components/RunArtifactsCard'
import { RunCheckpointCard } from './components/RunCheckpointCard'
import { RunConsoleCard } from './components/RunConsoleCard'
import { RunContextCard } from './components/RunContextCard'
import { RunEventTimelineCard } from './components/RunEventTimelineCard'
import { RunGraphCard } from './components/RunGraphCard'
import { RunList } from './components/RunList'
import { RunSummaryCard } from './components/RunSummaryCard'
import type { RunRecord } from './model/shared'
import { InlineNotice } from '@/ui'

export function RunsPanel() {
    const isNarrowViewport = useNarrowViewport()
    const [scopeMode, setScopeMode] = useState<'active' | 'all'>('active')
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const selectedRunId = useStore((state) => state.selectedRunId)
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
        summary,
    } = useRunsList({
        activeProjectPath,
        scopeMode,
        selectedRunId,
        viewMode,
    })
    const { requestCancel } = useRunActions({
        fetchRuns,
        setRuns,
    })
    const hasScopedSelectedRun = selectedRunId
        ? scopedRuns.some((run) => run.run_id === selectedRunId)
        : false
    const selectedRun =
        selectedRunSummary
            ?? (
                selectedRunRecord
                && selectedRunRecord.run_id === selectedRunId
                && (isLoading || Boolean(error) || hasScopedSelectedRun || scopedRuns.length === 0)
                    ? selectedRunRecord
                    : null
            )
    const selectedRunTimelineId = selectedRun?.run_id ?? null
    const {
        artifactDownloadHref,
        artifactEntries,
        artifactError,
        artifactViewerError,
        artifactViewerPayload,
        checkpointCompletedNodes,
        checkpointCurrentNode,
        checkpointData,
        checkpointError,
        checkpointRetryCounters,
        contextCopyStatus,
        contextError,
        contextExportHref,
        contextSearchQuery,
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
        viewMode,
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
        viewMode,
    })
    const degradedRunPanels = timelineError
        ? [...degradedDetailPanels, 'event timeline']
        : degradedDetailPanels
    const showRunSelectionEmptyState =
        ((scopeMode === 'active' && activeProjectPath) || scopeMode === 'all')
        && scopedRuns.length > 0
        && !selectedRun

    const selectRun = (run: RunRecord) => {
        setSelectedRunId(run.run_id)
        setSelectedRunSnapshot({ record: run, completedNodes: [] })
    }

    useEffect(() => {
        if (!selectedRunId) {
            return
        }
        if (selectedRunSummary && (!selectedRunRecord || selectedRunRecord.run_id !== selectedRunId)) {
            setSelectedRunSnapshot({ record: selectedRunSummary, completedNodes: [] })
        }
    }, [selectedRunId, selectedRunRecord, selectedRunSummary, setSelectedRunSnapshot])

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
                    onScopeModeChange={setScopeMode}
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
                        {selectedRun && (
                            <RunSummaryCard
                                activeProjectPath={activeProjectPath}
                                now={now}
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
                            <RunGraphCard
                                run={selectedRun}
                            />
                        )}
                        {selectedRun && (
                            <RunConsoleCard />
                        )}
                        {selectedRun && (
                            <RunCheckpointCard
                                checkpointCompletedNodes={checkpointCompletedNodes}
                                checkpointCurrentNode={checkpointCurrentNode}
                                checkpointData={checkpointData?.checkpoint ?? null}
                                checkpointError={checkpointError}
                                checkpointRetryCounters={checkpointRetryCounters}
                                isLoading={isCheckpointLoading}
                                onRefresh={() => {
                                    void fetchCheckpoint()
                                }}
                            />
                        )}
                        {selectedRun && (
                            <RunContextCard
                                contextCopyStatus={contextCopyStatus}
                                contextError={contextError}
                                contextExportHref={contextExportHref || null}
                                filteredContextRows={filteredContextRows}
                                isLoading={isContextLoading}
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
                            />
                        )}
                        {selectedRun && (
                            <RunArtifactsCard
                                artifactDownloadHref={(artifactPath) => artifactDownloadHref(artifactPath) || null}
                                artifactEntries={artifactEntries}
                                artifactError={artifactError}
                                artifactViewerError={artifactViewerError}
                                artifactViewerPayload={artifactViewerPayload || null}
                                isArtifactViewerLoading={isArtifactViewerLoading}
                                isLoading={isArtifactLoading}
                                missingCoreArtifacts={missingCoreArtifacts}
                                onRefresh={() => {
                                    void fetchArtifacts()
                                }}
                                onViewArtifact={(artifact) => {
                                    void viewArtifact(artifact)
                                }}
                                selectedArtifactEntry={selectedArtifactEntry}
                                showPartialRunArtifactNote={showPartialRunArtifactNote}
                            />
                        )}
                        {selectedRun && (
                            <RunEventTimelineCard
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
                            />
                        )}
                    </div>
                </div>
            </div>
        </section>
    )
}
