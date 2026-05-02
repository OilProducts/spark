import { useEffect, useRef, useState } from 'react'

import { retryLastSaveContent } from '@/lib/flowPersistence'
import { type PipelineStatusResponse } from '@/lib/attractorClient'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { useStore, type NodeStatus, type RuntimeStatus } from '@/store'
import { Button } from '@/components/ui/button'
import type { RunRecord } from './model/shared'
import { toTimelineEvent } from './model/timelineModel'
import { ApiHttpError, buildRunEventsUrl, loadSelectedRunJournal, loadSelectedRunStatus } from './services/runStreamTransport'
import { useRunsTransportReconnectSignal } from './services/runsTransportReconnect'
import {
    useRunJournalStore,
    type RunJournalStateEntry,
} from './state/runJournalStore'

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

function toRunRecord(status: PipelineStatusResponse): RunRecord {
    return {
        run_id: status.run_id,
        flow_name: status.flow_name || '',
        status: status.status,
        outcome: status.outcome ?? null,
        outcome_reason_code: status.outcome_reason_code ?? null,
        outcome_reason_message: status.outcome_reason_message ?? null,
        working_directory: status.working_directory || '',
        project_path: status.project_path,
        git_branch: status.git_branch ?? null,
        git_commit: status.git_commit ?? null,
        spec_id: status.spec_id ?? null,
        plan_id: status.plan_id ?? null,
        model: status.model || '',
        started_at: status.started_at || '',
        ended_at: status.ended_at ?? null,
        last_error: status.last_error || '',
        token_usage: typeof status.token_usage === 'number' || status.token_usage === null
            ? status.token_usage
            : undefined,
        token_usage_breakdown: status.token_usage_breakdown ?? undefined,
        estimated_model_cost: status.estimated_model_cost ?? undefined,
        current_node: status.current_node ?? status.progress?.current_node ?? null,
        continued_from_run_id: status.continued_from_run_id ?? null,
        continued_from_node: status.continued_from_node ?? null,
        continued_from_flow_mode: status.continued_from_flow_mode ?? null,
        continued_from_flow_name: status.continued_from_flow_name ?? null,
        parent_run_id: status.parent_run_id ?? null,
        parent_node_id: status.parent_node_id ?? null,
        root_run_id: status.root_run_id ?? null,
        child_invocation_index: status.child_invocation_index ?? null,
        execution_mode: status.execution_mode,
        execution_profile_id: status.execution_profile_id,
        execution_worker_id: status.execution_worker_id,
        execution_worker_label: status.execution_worker_label,
        execution_worker_base_url: status.execution_worker_base_url,
        execution_container_image: status.execution_container_image,
        execution_mapped_project_path: status.execution_mapped_project_path,
        execution_worker_runtime_root: status.execution_worker_runtime_root,
        execution_worker_version: status.execution_worker_version,
        execution_worker_capabilities: status.execution_worker_capabilities,
        execution_profile_capabilities: status.execution_profile_capabilities,
        cleanup_error: status.cleanup_error,
    }
}

function patchRunRecordFromRuntime(record: RunRecord, runtime: {
    status: RuntimeStatus
    outcome: 'success' | 'failure' | null
    outcomeReasonCode: string | null
    outcomeReasonMessage: string | null
    lastError?: string | null
}): RunRecord {
    return {
        ...record,
        status: runtime.status,
        outcome: runtime.outcome,
        outcome_reason_code: runtime.outcomeReasonCode,
        outcome_reason_message: runtime.outcomeReasonMessage,
        last_error: runtime.lastError ?? record.last_error,
    }
}

const SELECTED_RUN_JOURNAL_DEGRADED_MESSAGE =
    'Run journal history is unavailable. Reconnect to restore durable browsing for the selected run.'
const RUN_JOURNAL_PAGE_SIZE = 100

export function RunStream() {
    const setNodeStatus = useStore((state) => state.setNodeStatus)
    const setHumanGate = useStore((state) => state.setHumanGate)
    const clearHumanGate = useStore((state) => state.clearHumanGate)
    const resetNodeStatuses = useStore((state) => state.resetNodeStatuses)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const setRuntimeOutcome = useStore((state) => state.setRuntimeOutcome)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setSelectedRunSnapshot = useStore((state) => state.setSelectedRunSnapshot)
    const setSelectedRunStatusSync = useStore((state) => state.setSelectedRunStatusSync)
    const saveState = useStore((state) => state.saveState)
    const saveStateVersion = useStore((state) => state.saveStateVersion)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const resetSaveState = useStore((state) => state.resetSaveState)
    const reconnectSignal = useRunsTransportReconnectSignal(true)
    const [showSavedToast, setShowSavedToast] = useState(false)
    const [fadeSavedToast, setFadeSavedToast] = useState(false)
    const stageCursorsRef = useRef<Record<string, RuntimeStageCursor>>({})
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
        stageCursorsRef.current = {}
        resetNodeStatuses()
        clearHumanGate()
        if (!selectedRunId) {
            setSelectedRunSnapshot({ record: null, completedNodes: [], fetchedAtMs: null })
            setSelectedRunStatusSync('idle', null)
            setRuntimeStatus('idle')
            setRuntimeOutcome(null)
            return
        }
        setSelectedRunStatusSync('loading', null)
        setRuntimeStatus('idle')
        setRuntimeOutcome(null)
        useRunJournalStore.getState().patchRun(selectedRunId, {
            liveStatus: 'idle',
            liveError: null,
        })
    }, [
        clearHumanGate,
        resetNodeStatuses,
        selectedRunId,
        setRuntimeOutcome,
        setRuntimeStatus,
        setSelectedRunSnapshot,
        setSelectedRunStatusSync,
    ])

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

        let eventSource: EventSource | null = null
        let closed = false

        const closeStream = () => {
            eventSource?.close()
            eventSource = null
        }
        const patchRunJournal = (patch: Partial<RunJournalStateEntry>) => {
            useRunJournalStore.getState().patchRun(selectedRunId, patch)
        }

        const hasCachedSelectedRunSnapshot = () => {
            const state = useStore.getState()
            if (state.selectedRunRecord?.run_id === selectedRunId) {
                return true
            }
            return state.runDetailSessionsByRunId[selectedRunId]?.summaryRecord?.run_id === selectedRunId
        }

        const applyStatusSnapshot = (statusPayload: PipelineStatusResponse) => {
            const record = toRunRecord(statusPayload)
            setSelectedRunSnapshot({
                record,
                completedNodes: statusPayload.completed_nodes ?? [],
                fetchedAtMs: Date.now(),
            })
            setSelectedRunStatusSync('ready', null)
            if (isRuntimeStatus(record.status)) {
                setRuntimeStatus(record.status)
            }
            setRuntimeOutcome(
                record.outcome ?? null,
                record.outcome_reason_code ?? null,
                record.outcome_reason_message ?? null,
            )
        }

        const patchCurrentNode = (currentNode: string | null) => {
            const currentRecord = useStore.getState().selectedRunRecord
            if (currentRecord?.run_id !== selectedRunId) {
                return
            }
            if ((currentRecord.current_node ?? null) === currentNode) {
                return
            }
            setSelectedRunSnapshot({
                record: {
                    ...currentRecord,
                    current_node: currentNode,
                },
                completedNodes: useStore.getState().selectedRunCompletedNodes,
                fetchedAtMs: useStore.getState().selectedRunStatusFetchedAtMs,
            })
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
                if (closed) {
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
                if (closed) {
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
            try {
                const data = await loadSelectedRunStatus(selectedRunId)
                if (closed) {
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                applyStatusSnapshot(data)
                return {
                    payload: data,
                    shouldOpenStream: true,
                }
            } catch (error) {
                if (closed) {
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                if (error instanceof ApiHttpError && error.status === 404) {
                    setSelectedRunSnapshot({ record: null, completedNodes: [], fetchedAtMs: null })
                    setSelectedRunStatusSync('idle', null)
                    setSelectedRunId(null)
                    setRuntimeStatus('idle')
                    setRuntimeOutcome(null)
                    patchRunJournal({
                        liveStatus: 'idle',
                        liveError: null,
                    })
                    closeStream()
                    return {
                        payload: null,
                        shouldOpenStream: false,
                    }
                }
                const shouldOpenStream = hasCachedSelectedRunSnapshot()
                if (!shouldOpenStream) {
                    setSelectedRunStatusSync('degraded', SELECTED_RUN_STATUS_DEGRADED_MESSAGE)
                }
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

        const applyRuntimePatch = (runtimeStatus: RuntimeStatus, runtime: {
            outcome: 'success' | 'failure' | null
            outcomeReasonCode: string | null
            outcomeReasonMessage: string | null
            lastError?: string | null
        }) => {
            setRuntimeStatus(runtimeStatus)
            setRuntimeOutcome(runtime.outcome, runtime.outcomeReasonCode, runtime.outcomeReasonMessage)
            const currentRecord = useStore.getState().selectedRunRecord
            if (currentRecord?.run_id !== selectedRunId) {
                return
            }
            setSelectedRunSnapshot({
                record: patchRunRecordFromRuntime(currentRecord, {
                    status: runtimeStatus,
                    outcome: runtime.outcome,
                    outcomeReasonCode: runtime.outcomeReasonCode,
                    outcomeReasonMessage: runtime.outcomeReasonMessage,
                    lastError: runtime.lastError,
                }),
                completedNodes: useStore.getState().selectedRunCompletedNodes,
                fetchedAtMs: useStore.getState().selectedRunStatusFetchedAtMs,
            })
        }

        const handleMessage = (event: MessageEvent) => {
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
                        const currentGate = useStore.getState().humanGate
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
                            const currentGate = useStore.getState().humanGate
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
            if (closed || !shouldOpenStream) {
                return
            }
            const newestSequence = await refreshSelectedRunJournal()
            if (closed) {
                return
            }

            const nextSource = new EventSource(buildRunEventsUrl(selectedRunId, newestSequence))
            nextSource.onopen = () => {
                setSelectedRunStatusSync('ready', null)
                patchRunJournal({
                    liveStatus: 'live',
                    liveError: null,
                })
            }
            nextSource.onmessage = handleMessage
            nextSource.onerror = () => {
                if (closed) {
                    return
                }
                closeStream()
                setSelectedRunStatusSync('degraded', SELECTED_RUN_STATUS_DEGRADED_MESSAGE)
                patchRunJournal({
                    liveStatus: 'degraded',
                    liveError: SELECTED_RUN_STATUS_DEGRADED_MESSAGE,
                })
            }
            eventSource = nextSource
        }

        void startScopedStream()

        return () => {
            closed = true
            closeStream()
            patchRunJournal({
                liveStatus: 'idle',
            })
        }
    }, [
        clearHumanGate,
        reconnectSignal,
        resetNodeStatuses,
        selectedRunId,
        setHumanGate,
        setNodeStatus,
        setRuntimeOutcome,
        setRuntimeStatus,
        setSelectedRunId,
        setSelectedRunSnapshot,
        setSelectedRunStatusSync,
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
