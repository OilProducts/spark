import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { useShallow } from 'zustand/react/shallow'
import { selectSelectedRunId, selectSelectedRunSession } from '@/state/runsSessionSelectors'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import { LaunchPanel, loadRunSnapshotFlowContent, useFlowCatalog, type ContinuationDraft } from '@/features/launch'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useRunsList } from './hooks/useRunsList'
import { useRunActions } from './hooks/useRunActions'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { useRunTranscriptStore } from './state/runTranscriptStore'
import { flattenRunJournalSegments, useRunJournalStore } from './state/runJournalStore'
import { buildRunVisits, visitFlowNodesFromSnapshot, type VisitFlowNode } from './model/visitModel'
import { loadRunGraphPreview } from './services/runGraphTransport'
import { RunVisitsCard } from './components/RunVisitsCard'
import { RunGraphCard } from './components/RunGraphCard'
import { RunStatusItem } from './components/RunStatusItem'
import { RunContextItem } from './components/RunContextItem'
import { RunArtifactViewer } from './components/RunArtifactViewer'
import { RunList } from './components/RunList'
import { RunContinuationPanel } from './components/RunContinuationPanel'
import { RunHeaderBar } from './components/RunHeaderBar'
import { RunQuestionsPanel } from './components/RunQuestionsPanel'
import { type RunRecord } from './model/shared'
import { buildRunNodeStatuses } from './model/nodeStatusModel'
import { nodeOutcomesFromCheckpoint } from './model/runDetailsModel'
import { buildRunContextOverview, recordedGitRef, runStatusKind } from './model/runOverviewModel'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyDescription, EmptyHeader } from '@/components/ui/empty'
import { requestRunsTransportReconnect } from './services/runsTransportReconnect'
import { openRun } from '@/features/missions/MissionTranscript'

const EMPTY_NODE_STATUSES = {}
const EMPTY_FLOW_NODES: Record<string, VisitFlowNode> = {}

const STATUS_ROWS = {
    waiting: { label: 'Needs input', className: 'text-warning' },
    failed: { label: 'Failed', className: 'text-destructive' },
    completed: { label: 'Result' },
    running: { label: 'Running' },
    ended: { label: 'Status' },
}

const ACTIVE_RUN_STATUSES = new Set(['running', 'pause_requested', 'abort_requested', 'cancel_requested'])


function RunsSidebar({ activeProjectPath, scopeMode, selectedRunId }: {
    activeProjectPath: string | null
    scopeMode: 'active' | 'all'
    selectedRunId: string | null
}) {
    const { error, scopedRuns, status, summary } = useRunsList({
        activeProjectPath, scopeMode, selectedRunId, manageSync: false,
    })
    return (
        <RunList
            activeProjectPath={activeProjectPath}
            error={error}
            scopeMode={scopeMode}
            onScopeModeChange={(mode) => { useStore.getState().updateRunsListSession({ scopeMode: mode }); completePreferenceInteraction({ runs_scope: mode }) }}
            status={status}
            onSelectRun={(run) => {
                const state = useStore.getState()
                state.setRunsSelectedRunIdForScope(buildRunsScopeKey(scopeMode, activeProjectPath), run.run_id)
                state.reconcileRunRecord(run.run_id, 'list', run)
                // A run opens on its status item, not on a node picked last time.
                state.updateRunDetailSession(run.run_id, { selectedNodeId: null })
            }}
            runs={scopedRuns}
            selectedRunId={selectedRunId}
            summaryLabel={`${summary.total} runs · ${summary.running} running${summary.queued > 0 ? ` · ${summary.queued} queued` : ''}`}
        />
    )
}

export function RunsPanel() {
    const isNarrowViewport = useNarrowViewport()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const { scopeMode, status, streamError, streamStatus, hasRuns } = useStore(useShallow((state) => ({
        scopeMode: state.runsListSession.scopeMode,
        status: state.runsListSession.status,
        streamError: state.runsListSession.streamError,
        streamStatus: state.runsListSession.streamStatus,
        hasRuns: state.runsListSession.runs.length > 0,
    })))
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const selectedRunId = useStore(selectSelectedRunId)
    const selectedRunStatusSync = useStore((state) => selectSelectedRunSession(state)?.statusSync ?? 'idle')
    const selectedRunStatusError = useStore((state) => selectSelectedRunSession(state)?.statusError ?? null)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setViewMode = useStore((state) => state.setViewMode)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const setPendingEditorNodeSelection = useStore((state) => state.setPendingEditorNodeSelection)
    const flowCatalog = useFlowCatalog(Boolean(selectedRunId))
    const [continuationDraft, setContinuationDraft] = useState<ContinuationDraft | null>(null)
    const [rerunRun, setRerunRun] = useState<RunRecord | null>(null)
    const selectedRunSummary = useStore((state) => (
        state.runsListSession.runs.find((run) => run.run_id === selectedRunId) ?? null
    ))
    const { requestCancel, requestRetry } = useRunActions()
    const selectedRunDetailSession = useStore((state) => (
        selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? null : null
    ))
    const selectedRun = selectedRunDetailSession?.record ?? selectedRunSummary
    const selectedRunTimelineId = selectedRun?.run_id ?? null
    const {
        artifactDownloadHref,
        artifactEntries,
        artifactViewerError,
        artifactViewerPayload,
        checkpointCurrentNode,
        checkpointData,
        contextCopyStatus,
        contextData,
        contextError,
        contextExportHref,
        contextSearchQuery,
        contextStatus,
        degradedDetailPanels,
        fetchContext,
        isArtifactViewerLoading,
        pendingQuestionSnapshots,
        resultData,
        resultError,
        selectedArtifactEntry,
        setContextCopyStatus,
        setContextSearchQuery,
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
    const transcriptError = transcriptState?.status === 'error' ? transcriptState.error : null
    const {
        confirmedQuestionIds,
        freeformAnswersByGateId,
        gateNotesByGateId,
        groupedPendingInterviewGates,
        hasOlderTimelineEvents,
        isTimelineLive,
        isTimelineLoadingOlder,
        loadOlderTimelineEvents,
        pendingGateActionError,
        setFreeformAnswersByGateId,
        setGateNotesByGateId,
        submittingGateIds,
        submitPendingGateAnswer,
        timelineError,
        visiblePendingInterviewGates,
    } = useRunTimeline({
        pendingQuestionSnapshots,
        selectedRunTimelineId,
    })
    const selectedRunSessionState = useStore((state) => (
        selectedRun?.run_id ? state.runDetailSessionsByRunId[selectedRun.run_id] ?? null : null
    ))
    const degradedRunPanels = timelineError
        ? [...degradedDetailPanels, 'run journal']
        : degradedDetailPanels
    const showRunSelectionEmptyState =
        status === 'ready'
        && !selectedRunId
        && (((scopeMode === 'active' && activeProjectPath) || scopeMode === 'all'))
        && hasRuns
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
    const [artifactViewerPath, setArtifactViewerPath] = useState<string | null>(null)
    const openArtifact = useCallback((entry: { path: string; viewable: boolean }) => {
        setArtifactViewerPath(entry.path)
        void viewArtifact(entry)
    }, [viewArtifact])
    const detailsScrollRef = useRef<HTMLDivElement | null>(null)
    const currentNodeForSummary = selectedRun?.current_node || checkpointResumeNode
    const selectedNodeId = selectedRunSessionState?.selectedNodeId ?? null
    const liveNodeStatuses = useStore((state) => selectSelectedRunSession(state)?.nodeStatuses ?? EMPTY_NODE_STATUSES)
    const humanGateNodeId = useStore((state) => selectSelectedRunSession(state)?.humanGate?.nodeId ?? null)
    const gateNodeId = visiblePendingInterviewGates[0]?.nodeId ?? humanGateNodeId
    const isSelectedRunActive = selectedRun ? ACTIVE_RUN_STATUSES.has(selectedRun.status) : false
    const completedNodesSnapshot = selectedRunSessionState?.completedNodesSnapshot
    const checkpointNodeOutcomes = useMemo(
        () => nodeOutcomesFromCheckpoint(checkpointData),
        [checkpointData],
    )
    const selectedRunStatus = selectedRun?.status ?? null
    const journalSegments = useRunJournalStore((state) => (
        selectedRunTimelineId ? state.byRunId[selectedRunTimelineId]?.segments : undefined
    ))
    const journal = useMemo(
        () => (journalSegments ? flattenRunJournalSegments(journalSegments).reverse() : []),
        [journalSegments],
    )
    // Visits need the whole journal for timing, child runs and branches.
    // ponytail: loads every older page up front; page lazily if journals grow large.
    useEffect(() => {
        if (hasOlderTimelineEvents && !isTimelineLoadingOlder && !timelineError) {
            void loadOlderTimelineEvents()
        }
    }, [hasOlderTimelineEvents, isTimelineLoadingOlder, loadOlderTimelineEvents, timelineError])
    const [flowSnapshotsByRunId, setFlowSnapshotsByRunId] = useState<Record<string, { nodes: Record<string, VisitFlowNode>; title: string | null }>>({})
    useEffect(() => {
        if (!selectedRunTimelineId || flowSnapshotsByRunId[selectedRunTimelineId]) {
            return
        }
        const controller = new AbortController()
        loadRunGraphPreview(selectedRunTimelineId, { signal: controller.signal })
            .then((preview) => {
                const title = preview.flow?.title
                setFlowSnapshotsByRunId((current) => ({
                    ...current,
                    [selectedRunTimelineId]: {
                        nodes: visitFlowNodesFromSnapshot(preview.flow),
                        title: typeof title === 'string' && title.trim() ? title : null,
                    },
                }))
            })
            .catch(() => {
                // Without the snapshot, visits fall back to node ids for labels.
            })
        return () => controller.abort()
    }, [flowSnapshotsByRunId, selectedRunTimelineId])
    const flowSnapshot = selectedRunTimelineId ? flowSnapshotsByRunId[selectedRunTimelineId] : undefined
    const flowNodes = flowSnapshot?.nodes ?? EMPTY_FLOW_NODES
    const waitingNodeIds = useMemo(
        () => [
            ...visiblePendingInterviewGates.map((gate) => (gate.sourceScope === 'child' ? gate.sourceParentNodeId : gate.nodeId)),
            humanGateNodeId,
        ]
            .filter((nodeId): nodeId is string => Boolean(nodeId)),
        [humanGateNodeId, visiblePendingInterviewGates],
    )
    const visits = useMemo(() => (selectedRunTimelineId ? buildRunVisits({
        runId: selectedRunTimelineId,
        runStatus: selectedRunStatus,
        journal,
        executions: transcriptState?.executions ?? [],
        childRuns: transcriptState?.childRuns ?? [],
        flowNodes,
        waitingNodeIds,
    }) : []), [flowNodes, journal, selectedRunStatus, selectedRunTimelineId, transcriptState, waitingNodeIds])
    const finalContext = contextData?.context ?? null
    const launchContext = selectedRun?.launch_context
    const contextOverview = useMemo(
        () => buildRunContextOverview({ visits, launchContext, finalContext }),
        [finalContext, launchContext, visits],
    )
    const recordedRef = useMemo(() => recordedGitRef(contextOverview), [contextOverview])
    const statusKind = selectedRun ? runStatusKind(selectedRun.status, groupedPendingInterviewGates.length > 0) : 'ended'
    const selectedVisitLabel = selectedNodeId
        ? flowNodes[selectedNodeId]?.label ?? selectedNodeId
        : null
    const [showGraph, setShowGraph] = useState(false)
    useEffect(() => {
        const onKeyDown = (event: KeyboardEvent) => {
            const target = event.target instanceof Element ? event.target : null
            // The panel stays mounted behind other tabs; only Runs owns g.
            if (event.key !== 'g' || event.metaKey || event.ctrlKey || event.altKey || event.defaultPrevented
                || useStore.getState().viewMode !== 'runs'
                || target?.closest('input, textarea, select, [contenteditable="true"], [role="dialog"]')) {
                return
            }
            setShowGraph((current) => !current)
        }
        window.addEventListener('keydown', onKeyDown)
        return () => window.removeEventListener('keydown', onKeyDown)
    }, [])
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
        updateRunDetailSession(selectedRun.run_id, { selectedNodeId: nodeId })
    }, [selectedRun?.run_id, updateRunDetailSession])

    useEffect(() => {
        if (!selectedNodeId) {
            return
        }
        const onKeyDown = (event: KeyboardEvent) => {
            // Escape in a dialog (the file viewer) closes the dialog, not the selection.
            const target = event.target instanceof Element ? event.target : null
            if (event.key === 'Escape' && !target?.closest('[role="dialog"]')) {
                selectNode(null)
            }
        }
        window.addEventListener('keydown', onKeyDown)
        return () => {
            window.removeEventListener('keydown', onKeyDown)
        }
    }, [selectedNodeId, selectNode])

    const beginContinuation = (run: RunRecord) => {
        const projectPath = run.project_path || run.working_directory || null
        const normalizedModel = run.model === 'codex default (config/profile)' ? '' : run.model || ''

        if (projectPath) {
            setActiveProjectPath(projectPath)
        }
        setContinuationDraft({
            sourceRunId: run.run_id,
            sourceFlowName: run.flow_name || null,
            sourceWorkingDirectory: run.working_directory || projectPath || '',
            sourceModel: run.model || null,
            flowSourceMode: 'snapshot',
            // "Continue from here" restarts at the selected visit's node.
            startNodeId: selectedNodeId,
            workingDir: run.working_directory || projectPath || '',
            model: normalizedModel,
            overrideFlowName: run.flow_name || null,
        })
    }
    const activeContinuationDraft = continuationDraft && selectedRun?.run_id === continuationDraft.sourceRunId
        ? continuationDraft
        : null

    // A draft belongs to its source run; selecting a different run discards it.
    useEffect(() => {
        if (continuationDraft && selectedRun && selectedRun.run_id !== continuationDraft.sourceRunId) {
            setContinuationDraft(null)
        }
    }, [continuationDraft, selectedRun])

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
                        className="border-0 border-l border-warning px-3 py-2 text-warning"
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
                <RunsSidebar
                    activeProjectPath={activeProjectPath}
                    scopeMode={scopeMode}
                    selectedRunId={selectedRunId}
                />
                <div className={`min-w-0 ${isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 flex-col overflow-hidden pl-6'}`}>
                    <div
                        className={isNarrowViewport ? 'space-y-6' : 'flex min-h-0 flex-1 flex-col gap-4'}
                    >
                        {showRunSelectionEmptyState && (
                            <div data-testid="run-selection-empty-state" className="rounded-md border border-border px-3 py-2 text-sm text-muted-foreground">
                                Select a run from the sidebar to inspect its details.
                            </div>
                        )}
                        {showRunDetailsRestoringState && (
                            <p data-testid="run-selection-restoring-state" className="text-sm text-muted-foreground" aria-live="polite">Restoring the selected run session…</p>
                        )}
                        {selectedRun && (
                            <RunHeaderBar
                                run={selectedRun}
                                now={now}
                                flowTitle={flowSnapshot?.title ?? null}
                                selectedVisitLabel={selectedVisitLabel}
                                recordedCommit={recordedRef.commit}
                                onContinueFromRun={beginContinuation}
                                onRerunRun={(run) => {
                                    const projectPath = run.project_path || run.working_directory || null
                                    if (projectPath) {
                                        setActiveProjectPath(projectPath)
                                    }
                                    setRerunRun(run)
                                }}
                                onRequestCancel={(runId, currentStatus) => {
                                    void requestCancel(runId, currentStatus)
                                }}
                                onRequestRetry={(runId, currentStatus) => {
                                    void requestRetry(runId, currentStatus)
                                }}
                            />
                        )}
                        {selectedRun && activeContinuationDraft && (
                            <RunContinuationPanel
                                draft={activeContinuationDraft}
                                activeProjectPath={activeProjectPath}
                                onDraftChange={(patch) => {
                                    setContinuationDraft((draft) => (draft ? { ...draft, ...patch } : draft))
                                }}
                                onCancel={() => setContinuationDraft(null)}
                                onContinued={() => setContinuationDraft(null)}
                            />
                        )}
                        {selectedRun && degradedRunPanels.length > 0 && (
                            <div
                                data-testid="run-partial-api-failure-banner"
                                className="rounded-md border-0 border-l border-warning px-3 py-2 text-sm text-warning"
                            >
                                Some run detail endpoints are unavailable. Non-dependent panels remain functional.
                                <span className="ml-1 text-xs">
                                    Affected surfaces: {degradedRunPanels.join(', ')}.
                                </span>
                            </div>
                        )}
                        {!selectedRun && scopeMode === 'all' && !hasRuns && (
                            <Empty className="text-sm text-muted-foreground">
                                <EmptyHeader>
                                    <EmptyDescription>No runs have been recorded yet.</EmptyDescription>
                                </EmptyHeader>
                            </Empty>
                        )}
                        {selectedRun && (
                            <div className={isNarrowViewport
                                ? 'space-y-6'
                                : 'flex min-h-0 flex-1 gap-4'}
                            >
                                {/* A continuation picks its restart node on the graph. */}
                                {showGraph || activeContinuationDraft ? (
                                <div className={isNarrowViewport
                                    ? undefined
                                    : 'flex w-[24rem] min-w-[19rem] shrink-0 flex-col min-h-0 2xl:w-[28rem]'}
                                >
                                    <RunGraphCard
                                        key={`graph-${selectedRun.run_id}`}
                                        run={selectedRun}
                                        nodeStatusesById={runNodeStatuses}
                                        selectedNodeId={activeContinuationDraft ? activeContinuationDraft.startNodeId : selectedNodeId}
                                        onSelectNode={activeContinuationDraft
                                            ? (nodeId) => {
                                                setContinuationDraft((draft) => (draft ? { ...draft, startNodeId: nodeId } : draft))
                                            }
                                            : selectNode}
                                        onOpenInEditor={() => {
                                            const flowName = selectedRun.flow_name
                                            if (!flowName || !flowCatalog.flows.includes(flowName)) {
                                                return
                                            }
                                            setActiveFlow(flowName)
                                            setPendingEditorNodeSelection({
                                                flowName,
                                                nodeId: selectedNodeId ?? currentNodeForSummary ?? null,
                                            })
                                            setViewMode('editor')
                                        }}
                                        openInEditorDisabledReason={
                                            selectedRun.flow_name && flowCatalog.flows.includes(selectedRun.flow_name)
                                                ? null
                                                : flowCatalog.isLoaded
                                                    ? "This run's flow is not installed in the catalog."
                                                    : 'Loading the flow catalog…'
                                        }
                                        fillHeight={!isNarrowViewport}
                                    />
                                </div>
                                ) : null}
                                <div className={isNarrowViewport
                                    ? 'mt-6'
                                    : 'flex min-h-0 min-w-0 flex-1 flex-col'}
                                >
                                <div
                                    ref={detailsScrollRef}
                                    data-testid="run-details-scroll-region"
                                    className={isNarrowViewport ? undefined : 'flex min-h-0 flex-1 flex-col'}
                                >
                                    <RunVisitsCard
                                        key={selectedRun.run_id}
                                        visits={visits}
                                        flowNodes={flowNodes}
                                        segments={transcriptState?.segments ?? []}
                                        prompts={transcriptState?.prompts ?? []}
                                        journal={journal}
                                        now={now}
                                        isNarrowViewport={isNarrowViewport}
                                        isLive={isTimelineLive && isSelectedRunActive}
                                        transcriptError={transcriptError}
                                        timelineError={timelineError}
                                        selectedNodeId={selectedNodeId}
                                        onSelectNode={selectNode}
                                        onOpenRun={openRun}
                                        artifactEntries={artifactEntries}
                                        onViewArtifact={openArtifact}
                                        toolbar={
                                            <button
                                                type="button"
                                                data-testid="run-graph-toggle"
                                                aria-pressed={showGraph}
                                                aria-keyshortcuts="g"
                                                title="Show or hide the run graph (g)"
                                                onClick={() => setShowGraph((current) => !current)}
                                                className="hover:text-foreground"
                                            >
                                                {showGraph ? 'Hide graph' : 'Graph'}
                                            </button>
                                        }
                                        statusRow={STATUS_ROWS[statusKind]}
                                        renderStatus={(selectVisit) => (
                                            <RunStatusItem
                                                run={selectedRun}
                                                kind={statusKind}
                                                now={now}
                                                visits={visits}
                                                flowNodes={flowNodes}
                                                result={resultData}
                                                resultError={resultError}
                                                artifactEntries={artifactEntries}
                                                recordedRef={recordedRef}
                                                onSelectVisit={selectVisit}
                                                onViewArtifact={openArtifact}
                                                question={
                                                    <RunQuestionsPanel
                                                        confirmedQuestionIds={confirmedQuestionIds}
                                                        freeformAnswersByGateId={freeformAnswersByGateId}
                                                        gateNotesByGateId={gateNotesByGateId}
                                                        groupedPendingInterviewGates={groupedPendingInterviewGates}
                                                        onFreeformAnswerChange={(questionId, value) => {
                                                            setFreeformAnswersByGateId((previous) => ({
                                                                ...previous,
                                                                [questionId]: value,
                                                            }))
                                                        }}
                                                        onGateNoteChange={(questionId, value) => {
                                                            setGateNotesByGateId((previous) => ({
                                                                ...previous,
                                                                [questionId]: value,
                                                            }))
                                                        }}
                                                        onSubmitPendingGateAnswer={(gate, selectedValue, note) => {
                                                            void submitPendingGateAnswer(gate, selectedValue, note)
                                                        }}
                                                        pendingGateActionError={pendingGateActionError}
                                                        submittingGateIds={submittingGateIds}
                                                    />
                                                }
                                            />
                                        )}
                                        renderContext={(focusKey, selectVisit) => (
                                            <RunContextItem
                                                overview={contextOverview}
                                                finalContext={finalContext}
                                                status={contextStatus}
                                                contextError={contextError}
                                                searchQuery={contextSearchQuery}
                                                onSearchQueryChange={setContextSearchQuery}
                                                contextCopyStatus={contextCopyStatus}
                                                contextExportHref={contextExportHref || null}
                                                onCopy={() => {
                                                    void copyContextToClipboard()
                                                }}
                                                onRefresh={() => {
                                                    setContextCopyStatus('')
                                                    void fetchContext()
                                                }}
                                                focusKey={focusKey}
                                                onSelectVisit={selectVisit}
                                            />
                                        )}
                                    />
                                </div>
                                </div>
                            </div>
                        )}
                    </div>
                </div>
            </div>
            <RunArtifactViewer
                entry={selectedArtifactEntry?.path === artifactViewerPath ? selectedArtifactEntry : artifactViewerPath ? { path: artifactViewerPath } : null}
                open={artifactViewerPath !== null}
                onOpenChange={(open) => {
                    if (!open) {
                        setArtifactViewerPath(null)
                    }
                }}
                isLoading={isArtifactViewerLoading}
                error={artifactViewerError}
                payload={artifactViewerPayload || null}
                downloadHref={artifactViewerPath ? artifactDownloadHref(artifactViewerPath) || null : null}
            />
            <Dialog
                open={Boolean(rerunRun)}
                onOpenChange={(open) => {
                    if (!open) {
                        setRerunRun(null)
                    }
                }}
            >
                <DialogContent
                    data-testid="run-rerun-dialog"
                    className="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl"
                >
                    <DialogTitle className="sr-only">Re-run flow</DialogTitle>
                    {rerunRun ? (
                        <LaunchPanel
                            target={{
                                flowName: rerunRun.flow_name || null,
                                loadFlowContent: () => loadRunSnapshotFlowContent(rerunRun.run_id),
                                previewSource: {
                                    kind: 'runSnapshot',
                                    runId: rerunRun.run_id,
                                    displayName: rerunRun.flow_name || null,
                                },
                            }}
                            projectPath={rerunRun.project_path || rerunRun.working_directory || activeProjectPath}
                            initialLaunchContext={rerunRun.launch_context ?? null}
                            initialWorkingDirectory={rerunRun.working_directory || rerunRun.project_path || ''}
                            initialModel={rerunRun.model === 'codex default (config/profile)' ? '' : rerunRun.model || ''}
                            infoNotice={rerunRun.launch_context
                                ? null
                                : 'Original launch inputs were not recorded for this run; the form starts from the flow defaults.'}
                            onLaunched={() => {
                                setRerunRun(null)
                            }}
                            onClose={() => setRerunRun(null)}
                        />
                    ) : null}
                </DialogContent>
            </Dialog>
        </section>
    )
}
