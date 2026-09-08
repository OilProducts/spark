import { useEffect, useRef, useState } from 'react'

import { retryLastSaveContent } from '@/lib/flowPersistence'
import { type PipelineStatusResponse } from '@/lib/attractorClient'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { useStore, type NodeStatus, type RuntimeStatus } from '@/store'
import { Button } from '@/components/ui/button'
import { toRunRecord } from '@/state/runRecordReconciliation'
import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { toTimelineEvent } from './model/timelineModel'
import {
    ApiHttpError,
    loadRunTranscript,
    loadSelectedRunJournal,
    loadSelectedRunStatus,
    parseLiveRunTranscriptSegment,
} from './services/runStreamTransport'
import { useRunsTransportReconnectSignal } from './services/runsTransportReconnect'
import {
    useRunJournalStore,
    type RunJournalStateEntry,
} from './state/runJournalStore'
import { useRunTranscriptStore } from './state/runTranscriptStore'

const RUNTIME_STAGE_STATUS_MAP: Record<string, 'running' | 'success' | 'failed'> = {
    StageStarted: 'running',
    StageRetrying: 'running',
    StageCompleted: 'success',
    StageFailed: 'failed',
}

const SELECTED_RUN_STATUS_DEGRADED_MESSAGE =
    'Selected run live updates are unavailable. Reconnect to restore the selected run stream.'

interface RuntimeStageCursor {
    stageIndex: number
    status: 'running' | 'success' | 'failed'
    pendingRetry: boolean
}

const RUNTIME_STATUS_SET = new Set<RuntimeStatus>([
    'idle',
    'running',
    'abort_requested',
    'cancel_requested',
    'aborted',
    'canceled',
    'failed',
    'validation_error',
    'completed',
])

function isRuntimeStatus(value: string): value is RuntimeStatus {
    return RUNTIME_STATUS_SET.has(value as RuntimeStatus)
}

const NODE_STATUS_SET = new Set<NodeStatus>([
    'idle',
    'running',
    'success',
    'failed',
    'waiting',
])

function isNodeStatus(value: string): value is NodeStatus {
    return NODE_STATUS_SET.has(value as NodeStatus)
}

const SELECTED_RUN_JOURNAL_DEGRADED_MESSAGE =
    'Run journal history is unavailable. Reconnect to restore durable browsing for the selected run.'
const RUN_JOURNAL_PAGE_SIZE = 100

export function RunStream() {
    const selectedRunId = useStore(selectSelectedRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const reconcileRunRecord = useStore((state) => state.reconcileRunRecord)
    const clearRunDetailSession = useStore((state) => state.clearRunDetailSession)
    const saveState = useStore((state) => state.saveState)
    const saveStateVersion = useStore((state) => state.saveStateVersion)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const resetSaveState = useStore((state) => state.resetSaveState)
    const reconnectSignal = useRunsTransportReconnectSignal(true)
    const [showSavedToast, setShowSavedToast] = useState(false)
    const [fadeSavedToast, setFadeSavedToast] = useState(false)
    const savedToastFadeTimerRef = useRef<number | null>(null)
    const savedToastDismissTimerRef = useRef<number | null>(null)
    const saveStateLabel =
        saveState === 'saving'
            ? 'Saving...'
            : saveState === 'saved'
                ? 'Saved'
                : saveState === 'conflict'
                    ? 'Save Conflict'
                    : saveState === 'error'
                        ? 'Save Failed'
                        : ''
    const remediation = resolveSaveRemediation(saveState, saveErrorKind)
    const shouldShowPersistentSaveIndicator = saveState === 'saving' || saveState === 'error' || saveState === 'conflict'
    const showSaveStateIndicator = saveState === 'saved' || showSavedToast || shouldShowPersistentSaveIndicator
    const shouldFadeSaveCard = showSavedToast && !shouldShowPersistentSaveIndicator

    useEffect(() => {
        if (savedToastFadeTimerRef.current) {
            window.clearTimeout(savedToastFadeTimerRef.current)
            savedToastFadeTimerRef.current = null
        }
        if (savedToastDismissTimerRef.current) {
            window.clearTimeout(savedToastDismissTimerRef.current)
            savedToastDismissTimerRef.current = null
        }

        if (saveState !== 'saved') {
            setShowSavedToast(false)
            setFadeSavedToast(false)
            return
        }

        setShowSavedToast(true)
        setFadeSavedToast(false)
        savedToastFadeTimerRef.current = window.setTimeout(() => {
            setFadeSavedToast(true)
        }, 1000)
        savedToastDismissTimerRef.current = window.setTimeout(() => {
            setShowSavedToast(false)
            setFadeSavedToast(false)
            resetSaveState()
        }, 2000)

        return () => {
            if (savedToastFadeTimerRef.current) {
                window.clearTimeout(savedToastFadeTimerRef.current)
                savedToastFadeTimerRef.current = null
            }
            if (savedToastDismissTimerRef.current) {
                window.clearTimeout(savedToastDismissTimerRef.current)
                savedToastDismissTimerRef.current = null
            }
        }
    }, [resetSaveState, saveState, saveStateVersion])

    useEffect(() => {
        if (!selectedRunId) {
            return
        }

        let closed = false
        const stageCursorsRef: { current: Record<string, RuntimeStageCursor> } = { current: {} }
        const lastLiveSequenceRef: { current: number | null } = { current: null }
        const resyncInFlightRef = { current: false }
        let terminalRefreshRequested = false
        let pendingTerminalRefresh = false
        let statusRequestId = 0
        let needsOverlayHydration = true
        const liveNodeIds = new Set<string>()
        let liveGateReceived = false
        const lifetime = useStore.getState().runDetailSessionsByRunId[selectedRunId]?.lifetime
        const isCurrent = () => !closed && lifetime !== undefined && useStore.getState().runDetailSessionsByRunId[selectedRunId]?.lifetime === lifetime
        const setSelectedRunStatusSync = (statusSync: import('@/store').SelectedRunStatusSync, statusError: string | null = null) => {
            if (isCurrent()) updateRunDetailSession(selectedRunId, { statusSync, statusError })
        }
        const setNodeStatus = (nodeId: string, status: NodeStatus) => {
            liveNodeIds.add(nodeId)
            if (isCurrent()) updateRunDetailSession(selectedRunId, { nodeStatuses: {
                ...useStore.getState().runDetailSessionsByRunId[selectedRunId]?.nodeStatuses, [nodeId]: status,
            } })
        }
        const setHumanGate = (humanGate: import('@/store').HumanGateState | null) => {
            liveGateReceived = true
            if (isCurrent()) updateRunDetailSession(selectedRunId, { humanGate })
        }
        const clearHumanGate = () => setHumanGate(null)
        const resetNodeStatuses = () => {
            if (isCurrent()) updateRunDetailSession(selectedRunId, { nodeStatuses: {} })
        }

        const patchRunJournal = (patch: Partial<RunJournalStateEntry>) => {
            useRunJournalStore.getState().patchRun(selectedRunId, patch)
        }

        const hasCachedSelectedRunSnapshot = () => Boolean(useStore.getState().runDetailSessionsByRunId[selectedRunId]?.record)

        const patchCurrentNode = (currentNode: string | null) => {
            reconcileRunRecord(selectedRunId, 'journal', { current_node: currentNode })
        }

        const refreshSelectedRunJournal = async (): Promise<number | null> => {
            const currentJournalState = useRunJournalStore.getState().byRunId[selectedRunId]
            const hasCachedJournalEntries = Boolean(currentJournalState && currentJournalState.loadedEntryCount > 0)
            patchRunJournal({
                status: hasCachedJournalEntries ? currentJournalState!.status : 'loading',
                error: null,
            })
            try {
                const page = await loadSelectedRunJournal(selectedRunId, { limit: RUN_JOURNAL_PAGE_SIZE })
                if (!isCurrent()) {
                    return null
                }
                useRunJournalStore.getState().mergeLatestPage(selectedRunId, {
                    entries: page.entries
                        .map((entry) => toTimelineEvent(entry))
                        .filter((entry): entry is NonNullable<typeof entry> => entry !== null),
                    oldestSequence: page.oldest_sequence ?? null,
                    newestSequence: page.newest_sequence ?? null,
                    hasOlder: page.has_older,
                })
                return page.newest_sequence ?? useRunJournalStore.getState().byRunId[selectedRunId]?.newestSequence ?? null
            } catch (error) {
                if (!isCurrent()) {
                    return null
                }
                patchRunJournal({
                    status: hasCachedJournalEntries ? currentJournalState!.status : 'error',
                    error: error instanceof ApiHttpError
                        ? `Unable to load run journal (HTTP ${error.status})${error.detail ? `: ${error.detail}` : ''}.`
                        : SELECTED_RUN_JOURNAL_DEGRADED_MESSAGE,
                })
                return currentJournalState?.newestSequence ?? null
            }
        }

        const refreshSelectedRunStatus = async (): Promise<{
            payload: PipelineStatusResponse | null
            shouldOpenStream: boolean
        }> => {
            const requestId = ++statusRequestId
            const requestUpdates = useStore.getState().runDetailSessionsByRunId[selectedRunId]?.recordUpdates
            try {
                const data = await loadSelectedRunStatus(selectedRunId)
                if (!isCurrent() || requestId !== statusRequestId) {
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                // Keep cached overlays during loading/failure, but only overlays
                // received in this stream lifetime can outlive fresh hydration.
                if (needsOverlayHydration) {
                    const session = useStore.getState().runDetailSessionsByRunId[selectedRunId]
                    updateRunDetailSession(selectedRunId, {
                        nodeStatuses: Object.fromEntries(Object.entries(session.nodeStatuses).filter(([nodeId]) => liveNodeIds.has(nodeId))),
                        humanGate: liveGateReceived ? session.humanGate : null,
                    })
                    needsOverlayHydration = false
                }
                reconcileRunRecord(selectedRunId, 'status', toRunRecord(data), data.completed_nodes ?? [], requestUpdates)
                return {
                    payload: data,
                    shouldOpenStream: true,
                }
            } catch (error) {
                if (!isCurrent() || requestId !== statusRequestId) {
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                if (error instanceof ApiHttpError && error.status === 404) {
                    clearRunDetailSession(selectedRunId)
                    closed = true
                    patchRunJournal({
                        liveStatus: 'idle',
                        liveError: null,
                    })
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                const shouldOpenStream = hasCachedSelectedRunSnapshot()
                setSelectedRunStatusSync('degraded', SELECTED_RUN_STATUS_DEGRADED_MESSAGE)
                patchRunJournal({
                    liveStatus: 'degraded',
                    liveError: SELECTED_RUN_STATUS_DEGRADED_MESSAGE,
                })
                return {
                    payload: null,
                    shouldOpenStream,
                }
            }
        }

        const TERMINAL_RUN_STATUSES = new Set(['completed', 'failed', 'canceled', 'aborted'])

        const reconcileTerminalStatus = (status: string) => {
            if (!TERMINAL_RUN_STATUSES.has(status)) {
                terminalRefreshRequested = false
            } else if (!terminalRefreshRequested) {
                terminalRefreshRequested = true
                resyncFromDurableState(true)
            }
        }

        const applyRuntimePatch = (runtimeStatus: RuntimeStatus, runtime: {
            outcome: 'success' | 'failure' | null
            outcomeReasonCode: string | null
            outcomeReasonMessage: string | null
            lastError?: string | null
        }) => {
            reconcileRunRecord(selectedRunId, 'journal', {
                status: runtimeStatus,
                outcome: runtime.outcome,
                outcome_reason_code: runtime.outcomeReasonCode,
                outcome_reason_message: runtime.outcomeReasonMessage,
                ...(runtime.lastError != null ? { last_error: runtime.lastError } : {}),
            })
            // Coalesce both terminal event sources per execution attempt,
            // independently of other listeners' record writes.
            reconcileTerminalStatus(runtimeStatus)
        }

        const handleMessage = (event: { data: string }) => {
            if (!isCurrent()) return
            try {
                const data = JSON.parse(event.data)
                const rawRecord = (
                    data
                    && typeof data === 'object'
                    && !Array.isArray(data)
                ) ? data as Record<string, unknown> : {}
                const timelineEvent = toTimelineEvent(data)
                if (timelineEvent) {
                    useRunJournalStore.getState().appendLiveEntry(selectedRunId, timelineEvent)
                }
                const payload = (
                    timelineEvent?.payload
                    && typeof timelineEvent.payload === 'object'
                    && !Array.isArray(timelineEvent.payload)
                ) ? timelineEvent.payload : (
                    rawRecord.payload
                    && typeof rawRecord.payload === 'object'
                    && !Array.isArray(rawRecord.payload)
                ) ? rawRecord.payload as Record<string, unknown> : rawRecord
                const rawType = timelineEvent?.type
                    ?? (typeof rawRecord.raw_type === 'string' ? rawRecord.raw_type : typeof rawRecord.type === 'string' ? rawRecord.type : '')

                const liveSequence = timelineEvent?.sequence
                if (typeof liveSequence === 'number' && Number.isFinite(liveSequence)) {
                    const lastLiveSequence = lastLiveSequenceRef.current
                    if (lastLiveSequence !== null && liveSequence > lastLiveSequence + 1) {
                        resyncFromDurableState()
                    }
                    if (lastLiveSequence === null || liveSequence > lastLiveSequence) {
                        lastLiveSequenceRef.current = liveSequence
                    }
                }
                const runtimeNodeId = timelineEvent?.nodeId ?? (typeof payload.node_id === 'string' ? payload.node_id : null)
                const runtimeNodeStatus = RUNTIME_STAGE_STATUS_MAP[rawType]
                const runtimeStageIndex = timelineEvent?.stageIndex ?? (
                    typeof payload.index === 'number' && Number.isFinite(payload.index) ? payload.index : null
                )
                if (runtimeNodeId && runtimeNodeStatus) {
                    const previousCursor = stageCursorsRef.current[runtimeNodeId]
                    let shouldApplyStageStatus = true
                    if (runtimeStageIndex !== null && previousCursor) {
                        if (runtimeStageIndex < previousCursor.stageIndex) {
                            shouldApplyStageStatus = false
                        } else if (runtimeStageIndex === previousCursor.stageIndex) {
                            const previousIsTerminal = previousCursor.status === 'success' || previousCursor.status === 'failed'
                            if (runtimeNodeStatus === 'running' && previousCursor.status !== 'running') {
                                const retryContinuation = rawType === 'StageRetrying'
                                    || (rawType === 'StageStarted' && previousCursor.pendingRetry)
                                if (!retryContinuation) {
                                    shouldApplyStageStatus = false
                                }
                            }
                            if ((runtimeNodeStatus === 'success' || runtimeNodeStatus === 'failed') && previousIsTerminal && !previousCursor.pendingRetry) {
                                shouldApplyStageStatus = false
                            }
                        }
                    }

                    if (shouldApplyStageStatus) {
                        setNodeStatus(runtimeNodeId, runtimeNodeStatus)
                        if (runtimeNodeStatus === 'running') {
                            patchCurrentNode(runtimeNodeId)
                        }
                        if (runtimeStageIndex !== null) {
                            const stageAdvanced = previousCursor ? runtimeStageIndex > previousCursor.stageIndex : true
                            let pendingRetry = stageAdvanced ? false : (previousCursor?.pendingRetry ?? false)
                            if (rawType === 'StageFailed') {
                                pendingRetry = payload.will_retry === true
                            } else if (rawType === 'StageRetrying') {
                                pendingRetry = true
                            } else if (rawType === 'StageStarted' || rawType === 'StageCompleted') {
                                pendingRetry = false
                            }
                            stageCursorsRef.current[runtimeNodeId] = {
                                stageIndex: runtimeStageIndex,
                                status: runtimeNodeStatus,
                                pendingRetry,
                            }
                        }
                        const currentGate = useStore.getState().runDetailSessionsByRunId[selectedRunId]?.humanGate
                        if (currentGate?.nodeId === runtimeNodeId) {
                            clearHumanGate()
                        }
                    }
                }
                if (rawType === 'state' && payload.node && payload.status) {
                    const stateNodeId = typeof payload.node === 'string' ? payload.node : null
                    const stateNodeStatus =
                        typeof payload.status === 'string' && isNodeStatus(payload.status)
                            ? payload.status
                            : null
                    if (stateNodeId && stateNodeStatus) {
                        const previousCursor = stageCursorsRef.current[stateNodeId]
                        const stateRegression = stateNodeStatus === 'running'
                            && Boolean(previousCursor)
                            && previousCursor!.status !== 'running'
                            && !previousCursor!.pendingRetry
                        const stateTerminalFlip =
                            (stateNodeStatus === 'success' || stateNodeStatus === 'failed')
                            && Boolean(previousCursor)
                            && (previousCursor!.status === 'success' || previousCursor!.status === 'failed')
                            && !previousCursor!.pendingRetry
                        if (!stateRegression && !stateTerminalFlip) {
                            setNodeStatus(stateNodeId, stateNodeStatus)
                            if (stateNodeStatus === 'running') {
                                patchCurrentNode(stateNodeId)
                            }
                            if (previousCursor && (stateNodeStatus === 'running' || stateNodeStatus === 'success' || stateNodeStatus === 'failed')) {
                                stageCursorsRef.current[stateNodeId] = {
                                    ...previousCursor,
                                    status: stateNodeStatus,
                                    pendingRetry: stateNodeStatus === 'running' ? previousCursor.pendingRetry : false,
                                }
                            }
                            const currentGate = useStore.getState().runDetailSessionsByRunId[selectedRunId]?.humanGate
                            if (stateNodeStatus !== 'waiting' && currentGate?.nodeId === stateNodeId) {
                                clearHumanGate()
                            }
                        }
                    }
                }
                if (rawType === 'human_gate') {
                    const humanGateNodeId = typeof payload.node_id === 'string' ? payload.node_id : ''
                    setNodeStatus(humanGateNodeId, 'waiting')
                    patchCurrentNode(humanGateNodeId || null)
                    setHumanGate({
                        id: timelineEvent?.questionId ?? (typeof payload.question_id === 'string' ? payload.question_id : ''),
                        runId: selectedRunId,
                        nodeId: humanGateNodeId,
                        prompt: typeof payload.prompt === 'string' ? payload.prompt : '',
                        options: Array.isArray(payload.options) ? payload.options : [],
                        flowName: typeof payload.flow_name === 'string' ? payload.flow_name : undefined,
                    })
                }
                if (rawType === 'run_meta') {
                    resetNodeStatuses()
                    clearHumanGate()
                    patchCurrentNode(typeof payload.current_node === 'string' ? payload.current_node : null)
                    applyRuntimePatch('running', {
                        outcome: null,
                        outcomeReasonCode: null,
                        outcomeReasonMessage: null,
                    })
                }
                if (rawType === 'runtime' && typeof payload.status === 'string' && isRuntimeStatus(payload.status)) {
                    applyRuntimePatch(payload.status, {
                        outcome: payload.outcome === 'success' || payload.outcome === 'failure' ? payload.outcome : null,
                        outcomeReasonCode: typeof payload.outcome_reason_code === 'string' ? payload.outcome_reason_code : null,
                        outcomeReasonMessage: typeof payload.outcome_reason_message === 'string' ? payload.outcome_reason_message : null,
                        lastError: typeof payload.last_error === 'string' ? payload.last_error : null,
                    })
                }
            } catch {
                // Ignore malformed events.
            }
        }

        const startScopedStream = async () => {
            setSelectedRunStatusSync('loading', null)
            patchRunJournal({
                liveStatus: 'connecting',
                liveError: null,
            })
            const { shouldOpenStream } = await refreshSelectedRunStatus()
            if (!isCurrent() || !shouldOpenStream) {
                return
            }
            await refreshSelectedRunJournal()
            if (!isCurrent()) {
                return
            }

            if (useStore.getState().runDetailSessionsByRunId[selectedRunId]?.statusSync !== 'degraded') setSelectedRunStatusSync('ready', null)
            patchRunJournal({
                liveStatus: 'live',
                liveError: null,
            })
        }

        const handleLiveRunJournalEntry = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            if (detail?.runId !== selectedRunId || !detail.entry) {
                return
            }
            handleMessage({ data: JSON.stringify(detail.entry) })
        }

        const refreshRunTranscript = async () => {
            const transcript = useRunTranscriptStore.getState()
            if (transcript.byRunId[selectedRunId]?.status !== 'ready') {
                transcript.patchRun(selectedRunId, { status: 'loading', error: null })
            }
            try {
                const response = await loadRunTranscript(selectedRunId)
                if (!isCurrent()) {
                    return
                }
                useRunTranscriptStore.getState().setSegments(
                    selectedRunId,
                    response.segments,
                    response.newest_sequence,
                )
            } catch (error) {
                if (!isCurrent()) {
                    return
                }
                useRunTranscriptStore.getState().patchRun(selectedRunId, {
                    status: 'error',
                    error: error instanceof Error ? error.message : 'Unable to load run transcript.',
                })
            }
        }

        const handleRunSegmentUpsert = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            if (!isCurrent() || detail?.runId !== selectedRunId || !detail.segment) {
                return
            }
            const segment = parseLiveRunTranscriptSegment(detail.segment)
            if (segment) {
                useRunTranscriptStore.getState().applySegmentUpsert(selectedRunId, segment)
            }
        }

        // Live-derived node state may be built on dropped frames: clear the
        // overlay and rebuild everything from durable state. Deduped so a
        // burst of gap signals triggers one refetch cycle.
        const resyncFromDurableState = (terminal = false) => {
            if (!isCurrent()) return
            if (resyncInFlightRef.current) {
                pendingTerminalRefresh ||= terminal
                return
            }
            resyncInFlightRef.current = true
            stageCursorsRef.current = {}
            lastLiveSequenceRef.current = null
            resetNodeStatuses()
            clearHumanGate()
            void Promise.allSettled([
                refreshSelectedRunStatus(),
                refreshSelectedRunJournal(),
                refreshRunTranscript(),
            ]).finally(() => {
                resyncInFlightRef.current = false
                if (pendingTerminalRefresh && isCurrent()) {
                    pendingTerminalRefresh = false
                    resyncFromDurableState()
                }
            })
        }

        const handleRunResyncRequired = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            if (detail?.runId !== selectedRunId) {
                return
            }
            resyncFromDurableState()
        }

        // The finalize record write guarantees a terminal run.upsert even
        // when the journal frames around completion were dropped — and a
        // dropped terminal frame has no successor to expose the gap, so this
        // is the endgame backstop.
        const handleRunUpsert = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            const run = detail?.run as { run_id?: string; status?: string } | undefined
            if (!run || run.run_id !== selectedRunId || typeof run.status !== 'string') {
                return
            }
            if (!isCurrent()) return
            reconcileTerminalStatus(run.status)
        }

        void startScopedStream()
        void refreshRunTranscript()
        window.addEventListener('spark:run-journal-entry', handleLiveRunJournalEntry)
        window.addEventListener('spark:run-segment-upsert', handleRunSegmentUpsert)
        window.addEventListener('spark:run-resync-required', handleRunResyncRequired)
        window.addEventListener('spark:run-upsert', handleRunUpsert)

        return () => {
            closed = true
            window.removeEventListener('spark:run-upsert', handleRunUpsert)
            window.removeEventListener('spark:run-journal-entry', handleLiveRunJournalEntry)
            window.removeEventListener('spark:run-segment-upsert', handleRunSegmentUpsert)
            window.removeEventListener('spark:run-resync-required', handleRunResyncRequired)
            patchRunJournal({
                liveStatus: 'idle',
            })
        }
    }, [
        reconnectSignal,
        selectedRunId,
        updateRunDetailSession,
        reconcileRunRecord,
        clearRunDetailSession,
    ])

    const handleRetrySave = () => {
        void retryLastSaveContent()
    }

    return (
        <div data-testid="execution-runtime-stream-indicator" className="pointer-events-none fixed right-4 top-16 z-[70]">
            {showSaveStateIndicator ? (
                <div
                    data-testid="global-save-state-indicator"
                    className={`pointer-events-auto rounded-md border px-2 py-1 text-[11px] font-medium shadow-sm transition-opacity duration-1000 ${
                        saveState === 'error'
                            ? 'border-destructive/50 bg-destructive/10 text-destructive'
                            : saveState === 'conflict'
                                ? 'border-amber-500/50 bg-amber-500/10 text-amber-800'
                                : saveState === 'saved'
                                    ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-700'
                                    : 'border-border bg-background/95 text-muted-foreground'
                    } ${shouldFadeSaveCard && fadeSavedToast ? 'opacity-0' : 'opacity-100'}`}
                    title={saveErrorMessage || undefined}
                >
                    {saveStateLabel ? <span>{saveStateLabel}</span> : null}
                    {saveErrorMessage ? <span className="ml-1">- {saveErrorMessage}</span> : null}
                    {remediation ? (
                        <p data-testid="global-save-remediation-hint" className="mt-1 text-[10px] font-normal leading-4">
                            {remediation.message}
                        </p>
                    ) : null}
                    {remediation?.allowRetry ? (
                        <Button
                            data-testid="global-save-remediation-retry"
                            onClick={handleRetrySave}
                            variant="outline"
                            size="xs"
                            className="mt-2 border-current bg-transparent text-[10px] hover:bg-current/10"
                        >
                            Retry Save
                        </Button>
                    ) : null}
                </div>
            ) : null}
        </div>
    )
}
