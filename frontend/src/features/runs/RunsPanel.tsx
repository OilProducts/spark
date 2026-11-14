import { useShallow } from 'zustand/react/shallow'
import { InlineError } from '@/components/app/inline-error'
import { selectSelectedRunId, selectSelectedRunSession } from '@/state/runsSessionSelectors'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useStore } from '@/store'
import { LaunchPanel, openRunFlowDialog, useFlowCatalog, type ContinuationDraft } from '@/features/launch'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { cn } from '@/lib/utils'
import { useNow } from '@/lib/useNow'
import { useRunsList } from './hooks/useRunsList'
import { useRunActions } from './hooks/useRunActions'
import { useRunDetails } from './hooks/useRunDetails'
import { useRunTimeline } from './hooks/useRunTimeline'
import { useRunTranscriptStore } from './state/runTranscriptStore'
import { flattenRunJournalSegments, useRunJournalStore } from './state/runJournalStore'
import { buildRunVisits, visitFlowNodesFromSnapshot, type VisitFlowNode } from './model/visitModel'
import { loadRunGraphPreview } from './services/runGraphTransport'
import { RunVisitsCard } from './components/RunVisitsCard'
import { RunGraphCard, routeKey } from './components/RunGraphCard'
import { RunStatusItem } from './components/RunStatusItem'
import { RunContextItem } from './components/RunContextItem'
import { RunArtifactViewer } from './components/RunArtifactViewer'
import { RunList } from './components/RunList'
import { RunContinuationPanel } from './components/RunContinuationPanel'
import { focusRunHeading, RunHeaderBar } from './components/RunHeaderBar'
import { RunQuestionsPanel } from './components/RunQuestionsPanel'
import { formatDuration, runHasEnded, type RunRecord } from './model/shared'
import { buildRunNodeStatuses } from './model/nodeStatusModel'
import { nodeOutcomesFromCheckpoint } from './model/runDetailsModel'
import { buildRunContextOverview, runStatusKind, type RunStatusKind } from './model/runOverviewModel'
import type { StatusTone } from '@/components/app/status'
import { SheetEmpty, ViewLayout } from '@/components/app/view-layout'
import { orderProjects, projectLabel } from '@/features/projects/model/projectChoices'
import { Play, Plus, TriangleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { requestRunsTransportReconnect } from './services/runsTransportReconnect'
import { openRun } from '@/features/missions/MissionTranscript'
import { requestNavigation } from '@/state/workspaceSlice'

const EMPTY_NODE_STATUSES = {}
const EMPTY_FLOW_NODES: Record<string, VisitFlowNode> = {}
// The rail (16rem + gap) plus details and graph columns of 28rem each.
const RAIL_BESIDE_GRAPH_MIN_WIDTH = 1200

// The rail's first row; a dot only when the run needs attention.
const STATUS_ROWS: Record<RunStatusKind, { label: string; tone?: StatusTone }> = {
    waiting: { label: 'Needs input', tone: 'warning' },
    failed: { label: 'Failed', tone: 'destructive' },
    completed: { label: 'Result' },
    running: { label: 'Running', tone: 'info' },
    ended: { label: 'Status' },
}

const ACTIVE_RUN_STATUSES = new Set(['running', 'pause_requested', 'abort_requested', 'cancel_requested'])


function RunsSidebar({ selectedRunId }: { selectedRunId: string | null }) {
    const { error, scopedRuns, status } = useRunsList({ selectedRunId, manageSync: false })
    const projectFilter = useStore((state) => state.runsListSession.projectFilter)
    const statusFilter = useStore((state) => state.runsListSession.statusFilter ?? null)
    const searchQuery = useStore((state) => state.runsListSession.searchQuery ?? '')
    const runs = projectFilter ? scopedRuns.filter((run) => run.project_path === projectFilter) : scopedRuns
    return (
        <div className="flex h-full min-h-0 flex-col gap-2">
        <RunsProjectFilter />
        <RunList
            error={error}
            status={status}
            onSelectRun={(run) => {
                const state = useStore.getState()
                state.setRunsSelectedRunId(run.run_id)
                state.reconcileRunRecord(run.run_id, 'list', run)
            }}
            runs={runs}
            selectedRunId={selectedRunId}
            summaryLabel={runs.length === 0 ? 'runs' : `${runs.length} ${runs.length === 1 ? 'run' : 'runs'}`}
            // The filter already names the project; rows repeat it only across all projects.
            showProject={!projectFilter}
            statusFilter={statusFilter}
            onStatusFilterChange={(next) => useStore.getState().updateRunsListSession({ statusFilter: next })}
            searchQuery={searchQuery}
            onSearchQueryChange={(next) => useStore.getState().updateRunsListSession({ searchQuery: next })}
        />
        </div>
    )
}

function RunsProjectFilter() {
    const registry = useStore((state) => state.projectRegistry)
    const projectFilter = useStore((state) => state.runsListSession.projectFilter)
    // Headed as the Flows and Roles lists are: whose list this is, then the project to change it.
    return (
        <label className="flex min-w-0 items-center gap-2 text-sm text-muted-foreground [&>[data-slot=native-select-wrapper]]:min-w-0">
            <span className="shrink-0">Runs for</span>
            <NativeSelect
                size="sm"
                data-testid="runs-project-filter"
                aria-label="Show runs from project"
                value={projectFilter ?? ''}
                onChange={(event) => useStore.getState().updateRunsListSession({ projectFilter: event.target.value || null })}
                className="truncate bg-background pl-2.5 pr-8 font-medium text-foreground"
            >
                <NativeSelectOption value="">All projects</NativeSelectOption>
                {orderProjects(registry).map((project) => (
                    <NativeSelectOption key={project.directoryPath} value={project.directoryPath}>{projectLabel(registry, project.directoryPath)}</NativeSelectOption>
                ))}
            </NativeSelect>
        </label>
    )
}

export function RunsPanel() {
    const isNarrowViewport = useNarrowViewport()
    const { status, hasRuns } = useStore(useShallow((state) => ({
        status: state.runsListSession.status,
        hasRuns: state.runsListSession.runs.length > 0,
    })))
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const projectRegistry = useStore((state) => state.projectRegistry)
    const selectedRunId = useStore(selectSelectedRunId)
    const missingRunId = useStore((state) => state.runsListSession.missingRunId ?? null)
    const selectedRunStatusSync = useStore((state) => selectSelectedRunSession(state)?.statusSync ?? 'idle')
    const selectedRunStatusError = useStore((state) => selectSelectedRunSession(state)?.statusError ?? null)
    const setViewMode = useStore((state) => state.setViewMode)
    const isRunsView = useStore((state) => state.viewMode === 'runs')
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const setPendingEditorNodeSelection = useStore((state) => state.setPendingEditorNodeSelection)
    const [continuationDraft, setContinuationDraft] = useState<ContinuationDraft | null>(null)
    const [rerunRun, setRerunRun] = useState<RunRecord | null>(null)
    // A re-run's dialog closes onto the new run: focus goes to its heading, not to the page.
    const rerunLaunchedRef = useRef<string | null>(null)
    const selectedRunSummary = useStore((state) => (
        state.runsListSession.runs.find((run) => run.run_id === selectedRunId) ?? null
    ))
    const { requestCancel, requestRetry } = useRunActions()
    const selectedRunDetailSession = useStore((state) => (
        selectedRunId ? state.runDetailSessionsByRunId[selectedRunId] ?? null : null
    ))
    const selectedRun = selectedRunDetailSession?.record ?? selectedRunSummary
    const isSelectedRunLive = selectedRun ? !runHasEnded(selectedRun.status) : false
    // The editor opens a run's flow from the run's own project.
    const runProjectPath = selectedRun?.project_path ?? null
    const flowCatalog = useFlowCatalog(runProjectPath, Boolean(selectedRunId))
    const selectedRunTimelineId = selectedRun?.run_id ?? null
    const {
        artifactDownloadHref,
        artifactEntries,
        artifactError,
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
        runEnded: selectedRun ? runHasEnded(selectedRun.status) : false,
    })
    const selectedRunSessionState = useStore((state) => (
        selectedRun?.run_id ? state.runDetailSessionsByRunId[selectedRun.run_id] ?? null : null
    ))
    const degradedRunPanels = [
        ...(resultError ? ['result'] : []),
        ...(timelineError ? ['steps'] : []),
        ...(transcriptError ? ['agent transcripts'] : []),
        ...degradedDetailPanels,
    ]
    const showRunSelectionEmptyState = !selectedRunId && hasRuns && !selectedRun
    const showRunDetailsRestoringState =
        Boolean(selectedRunId)
        && !selectedRun
        && status !== 'ready'
        && status !== 'error'
    const now = useNow(isSelectedRunLive)
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
    const statusKind = selectedRun ? runStatusKind(selectedRun.status, groupedPendingInterviewGates.length > 0) : 'ended'
    const selectedVisitLabel = selectedNodeId
        ? flowNodes[selectedNodeId]?.label ?? selectedNodeId
        : null
    const [showGraph, setShowGraph] = useState(false)
    const takenRoutes = useMemo(
        () => new Set(visits.flatMap((visit) => (visit.decision ? [routeKey(visit.nodeId, visit.decision)] : []))),
        [visits],
    )
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
        if (!selectedRunTimelineId) {
            return
        }
        updateRunDetailSession(selectedRunTimelineId, { selectedNodeId: nodeId })
    }, [selectedRunTimelineId, updateRunDetailSession])

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

    // Stacked under the whole run list, a picked run's detail scrolls to the top.
    const runDetailRef = useRef<HTMLElement | null>(null)
    const [runBodyWidth, setRunBodyWidth] = useState(0)
    const runBodyObserverRef = useRef<ResizeObserver | null>(null)
    const runBodyRef = useCallback((element: HTMLDivElement | null) => {
        runBodyObserverRef.current?.disconnect()
        runBodyObserverRef.current = null
        if (!element || typeof ResizeObserver === 'undefined') {
            return
        }
        const observer = new ResizeObserver(([entry]) => setRunBodyWidth(entry.contentRect.width))
        observer.observe(element)
        runBodyObserverRef.current = observer
    }, [])
    useEffect(() => {
        if (isNarrowViewport && isRunsView && selectedRunId) {
            runDetailRef.current?.scrollIntoView({ block: 'start' })
        }
    }, [isNarrowViewport, isRunsView, selectedRunId])

    // A continuation picks its restart node on the graph, so it shows the graph too.
    const graphCard = selectedRun && (showGraph || activeContinuationDraft) ? (
        <RunGraphCard
            key={`graph-${selectedRun.run_id}`}
            run={selectedRun}
            nodeStatusesById={runNodeStatuses}
            takenRoutes={takenRoutes}
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
                const nodeId = selectedNodeId ?? currentNodeForSummary ?? null
                const open = () => {
                    useStore.getState().setActiveProjectPath(runProjectPath)
                    setActiveFlow(flowName)
                    setPendingEditorNodeSelection({ flowName, nodeId })
                    setViewMode('editor')
                }
                if (useStore.getState().projectPagePath) requestNavigation(open)
                else open()
            }}
            openInEditorDisabledReason={
                selectedRun.flow_name && flowCatalog.flows.includes(selectedRun.flow_name)
                    ? null
                    : flowCatalog.isLoaded
                        ? "This run's flow is not in its project's flows."
                        : 'Loading the project flows…'
            }
            fillHeight={!isNarrowViewport}
        />
    ) : null
    // With the graph open, the Steps rail keeps its own column only when the details and the
    // graph still get a readable column each; otherwise it sits above the details.
    const railBeside = runBodyWidth >= RAIL_BESIDE_GRAPH_MIN_WIDTH

    const runFlowAction = (
        <Button type="button" variant="outline" size="sm" data-testid="runs-empty-run-flow-button" onClick={() => openRunFlowDialog({ projectPath: useStore.getState().runsListSession.projectFilter })}>
            Run a flow…
        </Button>
    )

    return (
        <ViewLayout
            view="runs"
            title="Runs"
            actions={(
                <>
                    <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        data-testid="runs-run-flow-button"
                        aria-label="Run a flow"
                        title="Run a flow"
                        onClick={() => openRunFlowDialog({ projectPath: useStore.getState().runsListSession.projectFilter })}
                    >
                        <Plus aria-hidden="true" />
                    </Button>
                </>
            )}
            // ponytail: the run list mounts only while Runs shows, so live updates behind
            // other views don't regroup it; its search and expansion state resets on leaving.
            panel={isRunsView ? <RunsSidebar selectedRunId={selectedRunId} /> : null}
        >
        <section
            ref={runDetailRef}
            data-testid="runs-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            // On a phone the run grows past the sheet's screen height; it paints the sheet's white as it goes.
            className={isNarrowViewport ? 'min-h-full bg-background' : 'flex h-full min-h-0 flex-1 flex-col overflow-hidden'}
        >
            {/* The view header sits at the top of the sheet, under one hairline. */}
            {selectedRun && (
                <div className={`border-b border-border ${isNarrowViewport ? 'px-4 py-3' : 'px-6 py-4'}`}>
                    <RunHeaderBar
                        run={selectedRun}
                        now={now}
                        flowTitle={flowSnapshot?.title ?? null}
                        project={selectedRun.project_path ? projectLabel(projectRegistry, selectedRun.project_path) : null}
                        selectedVisitLabel={selectedVisitLabel}
                        continuationOpen={Boolean(activeContinuationDraft)}
                        onContinueFromRun={beginContinuation}
                        onRerunRun={(run) => setRerunRun(run)}
                        onRequestCancel={(runId, currentStatus) => {
                            void requestCancel(runId, currentStatus)
                        }}
                        onRequestRetry={(runId, currentStatus) => {
                            void requestRetry(runId, currentStatus)
                        }}
                    />
                </div>
            )}
            {/* A continuation's panel stacks above the run, which keeps its full height; the column
                scrolls between them, so the panel never squeezes the steps and the graph. */}
            <div className={isNarrowViewport ? 'space-y-6 p-4' : cn('flex min-h-0 flex-1 flex-col gap-4 px-6 py-5', activeContinuationDraft && 'overflow-y-auto')}>
                {/* One notice for whatever of this run couldn't load or stopped updating; the
                    parts it names say so in place instead of looking empty. */}
                {selectedRunStatusSync === 'degraded' || (selectedRun && degradedRunPanels.length > 0) ? (
                    <div
                        data-testid={selectedRunStatusSync === 'degraded' ? 'runs-transport-reconnect-banner' : 'run-partial-api-failure-banner'}
                        role="status"
                        className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border-0 border-l border-warning px-3 py-2 text-sm text-warning"
                    >
                        <span>
                            {selectedRunStatusSync === 'degraded' ? selectedRunStatusError : 'Some run details couldn\'t load.'}
                            {degradedRunPanels.length > 0 ? (
                                <span className="ml-1 text-xs">Not loaded: {degradedRunPanels.join(', ')}.</span>
                            ) : null}
                        </span>
                        <Button data-testid="runs-transport-reconnect-button" type="button" variant="outline" size="xs" onClick={requestRunsTransportReconnect}>Try again</Button>
                    </div>
                ) : null}
                {!selectedRunId && missingRunId ? (
                    <InlineError data-testid="run-missing" title="This run isn’t here">
                        There’s no run with the ID “{missingRunId}”. It may have been deleted, or the link is out of date.
                    </InlineError>
                ) : null}
                {showRunSelectionEmptyState && (
                    <SheetEmpty
                        data-testid="run-selection-empty-state"
                        icon={Play}
                        title="No run selected"
                        className="flex-1"
                        action={runFlowAction}
                    >
                        Pick a run from the list to see its progress and results, or start a new one.
                    </SheetEmpty>
                )}
                {showRunDetailsRestoringState && (
                    <p data-testid="run-selection-restoring-state" className="text-sm text-muted-foreground" aria-live="polite">Restoring the selected run session…</p>
                )}
                {selectedRun && activeContinuationDraft && (
                    <RunContinuationPanel
                        draft={activeContinuationDraft}
                        projectPath={selectedRun.project_path || null}
                        startNodeLabel={activeContinuationDraft.startNodeId
                            ? flowNodes[activeContinuationDraft.startNodeId]?.label ?? activeContinuationDraft.startNodeId
                            : null}
                        onDraftChange={(patch) => {
                            setContinuationDraft((draft) => (draft ? { ...draft, ...patch } : draft))
                        }}
                        onCancel={() => setContinuationDraft(null)}
                        onContinued={(runId) => {
                            setContinuationDraft(null)
                            if (runId) focusRunHeading(runId)
                        }}
                    />
                )}
                {/* Until the history loads, no runs says nothing; a failed load says so once, here. */}
                {!selectedRun && !hasRuns && status === 'error' && (
                    <SheetEmpty
                        data-testid="runs-load-error"
                        role="alert"
                        icon={TriangleAlert}
                        title="Couldn’t load your runs"
                        className="flex-1"
                        action={<Button type="button" variant="outline" size="sm" onClick={requestRunsTransportReconnect}>Try again</Button>}
                    >
                        Spark didn’t answer when asked for your run history. Check that it is still running, then try again.
                    </SheetEmpty>
                )}
                {!selectedRun && !hasRuns && status === 'ready' && (
                    <SheetEmpty icon={Play} title="No runs yet" className="flex-1" action={runFlowAction}>
                        Start a flow to see its progress and results here.
                    </SheetEmpty>
                )}
                {selectedRun && (
                    <div ref={runBodyRef} className={isNarrowViewport
                        ? 'space-y-6'
                        : cn('flex min-h-0 flex-1 gap-6', activeContinuationDraft && 'min-h-full')}
                    >
                        <div className={isNarrowViewport
                            ? undefined
                            : cn('flex min-h-0 min-w-0 flex-col', !graphCard ? 'flex-[1_1_0%]' : railBeside ? 'flex-[1_1_17.5rem]' : 'flex-[2_1_0%]')}
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
                                timelineError={timelineError}
                                selectedNodeId={selectedNodeId}
                                onSelectNode={selectNode}
                                onOpenRun={openRun}
                                artifactEntries={artifactEntries}
                                onViewArtifact={openArtifact}
                                graph={{ shown: showGraph, onToggle: () => setShowGraph((current) => !current) }}
                                railAbove={Boolean(graphCard) && !railBeside}
                                statusRow={{
                                    ...STATUS_ROWS[statusKind],
                                    detail: formatDuration(selectedRun.started_at, selectedRun.ended_at, selectedRun.status, now),
                                }}
                                contextKeyCount={contextError ? null : contextOverview.namespaces.reduce((count, namespace) => count + namespace.keys.length, 0)}
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
                                        artifactsUnavailable={Boolean(artifactError)}
                                        onSelectVisit={selectVisit}
                                        onViewArtifact={openArtifact}
                                        question={
                                            <RunQuestionsPanel
                                                confirmedQuestionIds={confirmedQuestionIds}
                                                freeformAnswersByGateId={freeformAnswersByGateId}
                                                gateNotesByGateId={gateNotesByGateId}
                                                groupedPendingInterviewGates={groupedPendingInterviewGates}
                                                now={now}
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
                        {/* After the details, so on a phone a question to answer comes first and the
                            rail's Graph toggle sits just above the graph; beside them it never moves the
                            Steps rail. A continuation picks its restart node on it. */}
                        {graphCard ? (
                        <div className={isNarrowViewport
                            ? undefined
                            // With the rail above the details, the graph takes the larger share.
                            : cn('flex min-h-0 min-w-0 flex-col', railBeside ? 'flex-[1_1_0%]' : 'flex-[3_1_0%]')}
                        >
                            {graphCard}
                        </div>
                        ) : null}
                    </div>
                )}
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
                    onCloseAutoFocus={(event) => {
                        const runId = rerunLaunchedRef.current
                        rerunLaunchedRef.current = null
                        if (runId) {
                            event.preventDefault()
                            focusRunHeading(runId)
                        }
                    }}
                >
                    <DialogTitle className="sr-only">Re-run flow</DialogTitle>
                    {rerunRun ? (
                        <LaunchPanel
                            target={{
                                flowName: rerunRun.flow_name || null,
                                // Re-run launches the installed flow by name, so its live inputs and defaults drive
                                // the form; the recorded launch context only seeds initial values.
                                previewSource: { kind: 'flow', flowName: rerunRun.flow_name },
                            }}
                            projectPath={rerunRun.project_path || rerunRun.working_directory || null}
                            initialLaunchContext={rerunRun.launch_context ?? null}
                            initialWorkingDirectory={rerunRun.working_directory || rerunRun.project_path || ''}
                            initialModel={rerunRun.model === 'codex default (config/profile)' ? '' : rerunRun.model || ''}
                            infoNotice={rerunRun.launch_context
                                ? null
                                : 'Original launch inputs were not recorded for this run; the form starts from the flow defaults.'}
                            onLaunched={(runId) => {
                                rerunLaunchedRef.current = runId
                                setRerunRun(null)
                            }}
                        />
                    ) : null}
                </DialogContent>
            </Dialog>
        </section>
        </ViewLayout>
    )
}
