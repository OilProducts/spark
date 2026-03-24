import { RefreshCcw } from 'lucide-react'
import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import {
    ApiHttpError,
    fetchPipelineCancelValidated,
} from '@/lib/attractorClient'
import { useRunsList } from './hooks/useRunsList'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { RunArtifactsCard } from './components/RunArtifactsCard'
import { RunCheckpointCard } from './components/RunCheckpointCard'
import { RunContextCard } from './components/RunContextCard'
import { RunEventTimelineCard } from './components/RunEventTimelineCard'
import { RunGraphvizCard } from './components/RunGraphvizCard'
import { RunList } from './components/RunList'
import { RunSummaryCard } from './components/RunSummaryCard'
import type { RunRecord } from './model/shared'
import { Button, InlineNotice, SectionHeader } from '@/ui'

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

export function RunsPanel() {
    const isNarrowViewport = useNarrowViewport()
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setSpecId = useStore((state) => state.setSpecId)
    const setPlanId = useStore((state) => state.setPlanId)
    const {
        error,
        fetchRuns,
        isLoading,
        metadataFreshness,
        metadataFreshnessLabel,
        metadataFreshnessStyle,
        now,
        scopedRuns,
        selectedRunSummary,
        setRuns,
        summary,
        updatedAtLabel,
    } = useRunsList({
        activeProjectPath,
        selectedRunId,
        viewMode,
    })
    const selectedRunTimelineId = selectedRunSummary?.run_id ?? null
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
        fetchGraphviz,
        filteredContextRows,
        graphvizError,
        graphvizViewerSrc,
        isArtifactLoading,
        isArtifactViewerLoading,
        isCheckpointLoading,
        isContextLoading,
        isGraphvizLoading,
        missingCoreArtifacts,
        pendingQuestionSnapshots,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
        showPartialRunArtifactNote,
        viewArtifact,
        copyContextToClipboard,
    } = useRunDetails({
        selectedRunSummary,
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

    const openRun = (run: RunRecord) => {
        setSelectedRunId(run.run_id)
        setExecutionFlow(run.flow_name || null)
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

    const requestCancel = async (runId: string, currentStatus: string) => {
        if (currentStatus !== 'running') {
            return
        }
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        setRuns((current) =>
            current.map((run) => (
                run.run_id === runId
                    ? { ...run, status: 'cancel_requested' }
                    : run
            ))
        )
        try {
            await fetchPipelineCancelValidated(runId)
            void fetchRuns()
        } catch (err) {
            logUnexpectedRunError(err)
            setRuns((current) =>
                current.map((run) => (
                    run.run_id === runId
                        ? { ...run, status: currentStatus }
                        : run
                ))
            )
            window.alert('Failed to cancel run')
        }
    }

    return (
        <div data-testid="runs-panel" className={`flex-1 overflow-auto ${isNarrowViewport ? 'p-3' : 'p-6'}`}>
            <div className="mx-auto w-full max-w-6xl space-y-6">
                <div className="flex items-center justify-between">
                    <SectionHeader
                        title="Run History"
                        description={`${summary.total} total runs · ${summary.running} running`}
                    />
                    <Button
                        onClick={() => void fetchRuns()}
                        data-testid="runs-refresh-button"
                        variant="outline"
                        size="sm"
                    >
                        <RefreshCcw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
                        Refresh
                    </Button>
                </div>
                <div className="flex flex-wrap items-center gap-2 text-xs">
                    <span
                        data-testid="run-metadata-freshness-indicator"
                        className={`inline-flex items-center rounded-md border px-2 py-1 font-semibold uppercase tracking-wide ${metadataFreshnessStyle}`}
                    >
                        {metadataFreshnessLabel}
                    </span>
                    <span data-testid="run-metadata-last-updated" className="text-muted-foreground">
                        {updatedAtLabel}
                    </span>
                </div>
                {metadataFreshness === 'stale' && (
                    <InlineNotice data-testid="run-metadata-stale-indicator" tone="warning">
                        Run metadata may be stale. Refresh to load the latest run status.
                    </InlineNotice>
                )}

                {error && (
                    <InlineNotice tone="error">
                        {error}
                    </InlineNotice>
                )}
                {!activeProjectPath && (
                    <InlineNotice>
                        Select an active project to view run history for that project.
                    </InlineNotice>
                )}
                {activeProjectPath && scopedRuns.length > 0 && !selectedRunSummary && (
                    <div data-testid="run-selection-empty-state" className="rounded-md border border-border bg-card px-3 py-2 text-sm text-muted-foreground">
                        Select a run from the history table to inspect its details.
                    </div>
                )}
                {selectedRunSummary && (
                    <RunSummaryCard
                        activeProjectPath={activeProjectPath}
                        now={now}
                        run={selectedRunSummary}
                    />
                )}
                {selectedRunSummary && degradedRunPanels.length > 0 && (
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
                {selectedRunSummary && (
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
                {selectedRunSummary && (
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
                        runId={selectedRunSummary.run_id}
                        searchQuery={contextSearchQuery}
                    />
                )}
                {selectedRunSummary && (
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
                {selectedRunSummary && (
                    <RunGraphvizCard
                        graphvizError={graphvizError}
                        graphvizViewerSrc={graphvizViewerSrc || null}
                        isGraphvizLoading={isGraphvizLoading}
                        onRefresh={() => {
                            void fetchGraphviz()
                        }}
                        runId={selectedRunSummary.run_id}
                    />
                )}
                {selectedRunSummary && (
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
                <RunList
                    activeProjectPath={activeProjectPath}
                    now={now}
                    onOpenRun={openRun}
                    onOpenRunArtifact={openRunArtifact}
                    onRequestCancel={(runId, currentStatus) => {
                        void requestCancel(runId, currentStatus)
                    }}
                    runs={scopedRuns}
                    selectedRunId={selectedRunId}
                />
            </div>
        </div>
    )
}
