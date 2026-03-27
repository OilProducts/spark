import { useEffect, useRef, useState } from 'react'

import { retryLastSaveContent } from '@/lib/flowPersistence'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { useStore, type RuntimeStatus } from '@/store'
import { Button } from '@/ui'

import { ApiHttpError, buildRunEventsUrl, loadSelectedRunStatus } from './services/runStreamTransport'

function classifyLog(message: string): 'info' | 'success' | 'error' {
    const lower = message.toLowerCase()
    const isSuccess = lower.includes('success')
    const isError = /fail|error|⚠️/i.test(message)
    if (isSuccess) return 'success'
    if (isError) return 'error'
    return 'info'
}

const RUNTIME_STAGE_STATUS_MAP: Record<string, 'running' | 'success' | 'failed'> = {
    StageStarted: 'running',
    StageRetrying: 'running',
    StageCompleted: 'success',
    StageFailed: 'failed',
}

const SELECTED_RUN_STATUS_DEGRADED_MESSAGE =
    'Selected run status endpoint is unavailable or incompatible. Live event streaming preflight is in degraded mode.'

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

export function RunStream() {
    const addLog = useStore((state) => state.addLog)
    const clearLogs = useStore((state) => state.clearLogs)
    const setNodeStatus = useStore((state) => state.setNodeStatus)
    const setHumanGate = useStore((state) => state.setHumanGate)
    const clearHumanGate = useStore((state) => state.clearHumanGate)
    const resetNodeStatuses = useStore((state) => state.resetNodeStatuses)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const setRuntimeOutcome = useStore((state) => state.setRuntimeOutcome)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const saveState = useStore((state) => state.saveState)
    const saveStateVersion = useStore((state) => state.saveStateVersion)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const resetSaveState = useStore((state) => state.resetSaveState)
    const [runtimeApiDegradedMessage, setRuntimeApiDegradedMessage] = useState<string | null>(null)
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
    const showSaveStateIndicator = (
        saveState === 'saved'
        || showSavedToast
        || shouldShowPersistentSaveIndicator
        || Boolean(runtimeApiDegradedMessage)
    )
    const shouldFadeSaveCard = showSavedToast && !shouldShowPersistentSaveIndicator && !runtimeApiDegradedMessage

    useEffect(() => {
        stageCursorsRef.current = {}
        resetNodeStatuses()
        clearHumanGate()
        clearLogs()
        setRuntimeStatus('idle')
        setRuntimeOutcome(null)
    }, [selectedRunId, resetNodeStatuses, clearHumanGate, clearLogs, setRuntimeStatus, setRuntimeOutcome])

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
            setRuntimeApiDegradedMessage(null)
            setRuntimeStatus('idle')
            return
        }

        let eventSource: EventSource | null = null
        const metadataAbort = new AbortController()
        const source = {
            close: () => {
                metadataAbort.abort()
                eventSource?.close()
            },
        }

        const handleMessage = (event: MessageEvent) => {
            try {
                const data = JSON.parse(event.data)
                const runtimeNodeId = typeof data.node_id === 'string' ? data.node_id : null
                const runtimeNodeStatus = RUNTIME_STAGE_STATUS_MAP[data.type]
                const runtimeStageIndex = typeof data.index === 'number' && Number.isFinite(data.index) ? data.index : null
                if (runtimeNodeId && runtimeNodeStatus) {
                    const previousCursor = stageCursorsRef.current[runtimeNodeId]
                    let shouldApplyStageStatus = true
                    if (runtimeStageIndex !== null && previousCursor) {
                        if (runtimeStageIndex < previousCursor.stageIndex) {
                            shouldApplyStageStatus = false
                        } else if (runtimeStageIndex === previousCursor.stageIndex) {
                            const previousIsTerminal = previousCursor.status === 'success' || previousCursor.status === 'failed'
                            if (runtimeNodeStatus === 'running' && previousCursor.status !== 'running') {
                                const retryContinuation = data.type === 'StageRetrying'
                                    || (data.type === 'StageStarted' && previousCursor.pendingRetry)
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
                        if (runtimeStageIndex !== null) {
                            const stageAdvanced = previousCursor ? runtimeStageIndex > previousCursor.stageIndex : true
                            let pendingRetry = stageAdvanced ? false : (previousCursor?.pendingRetry ?? false)
                            if (data.type === 'StageFailed') {
                                pendingRetry = data.will_retry === true
                            } else if (data.type === 'StageRetrying') {
                                pendingRetry = true
                            } else if (data.type === 'StageStarted' || data.type === 'StageCompleted') {
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
                if (data.type === 'log') {
                    addLog({
                        time: new Date().toLocaleTimeString('en-GB', { hour12: false }),
                        msg: data.msg,
                        type: classifyLog(data.msg),
                    })
                }
                if (data.type === 'state' && data.node && data.status) {
                    const stateNodeId = typeof data.node === 'string' ? data.node : null
                    const stateNodeStatus = typeof data.status === 'string' ? data.status : null
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
                if (data.type === 'human_gate') {
                    setNodeStatus(data.node_id, 'waiting')
                    setHumanGate({
                        id: data.question_id,
                        runId: selectedRunId,
                        nodeId: data.node_id,
                        prompt: data.prompt,
                        options: data.options || [],
                        flowName: data.flow_name,
                    })
                }
                if (data.type === 'run_meta') {
                    resetNodeStatuses()
                    clearHumanGate()
                    setRuntimeStatus('running')
                    setRuntimeOutcome(null)
                }
                if (data.type === 'runtime' && typeof data.status === 'string' && isRuntimeStatus(data.status)) {
                    setRuntimeStatus(data.status)
                    setRuntimeOutcome(
                        data.outcome === 'success' || data.outcome === 'failure' ? data.outcome : null,
                        typeof data.outcome_reason_code === 'string' ? data.outcome_reason_code : null,
                        typeof data.outcome_reason_message === 'string' ? data.outcome_reason_message : null,
                    )
                }
            } catch {
                // ignore malformed events
            }
        }

        const startScopedStream = async () => {
            let data: Awaited<ReturnType<typeof loadSelectedRunStatus>> | null = null
            if (!metadataAbort.signal.aborted) {
                try {
                    data = await loadSelectedRunStatus(selectedRunId)
                    setRuntimeApiDegradedMessage((current) =>
                        current === SELECTED_RUN_STATUS_DEGRADED_MESSAGE ? null : current
                    )
                } catch (error) {
                    if (!metadataAbort.signal.aborted && error instanceof ApiHttpError && error.status === 404) {
                        setSelectedRunId(null)
                        setRuntimeStatus('idle')
                        setRuntimeApiDegradedMessage(null)
                        return
                    }
                    if (!metadataAbort.signal.aborted) {
                        setRuntimeApiDegradedMessage(SELECTED_RUN_STATUS_DEGRADED_MESSAGE)
                    }
                    return
                }
            }
            if (metadataAbort.signal.aborted) return

            if (typeof data?.status === 'string' && isRuntimeStatus(data.status)) {
                setRuntimeStatus(data.status)
                setRuntimeOutcome(
                    data.outcome === 'success' || data.outcome === 'failure' ? data.outcome : null,
                    typeof data.outcome_reason_code === 'string' ? data.outcome_reason_code : null,
                    typeof data.outcome_reason_message === 'string' ? data.outcome_reason_message : null,
                )
            }
            if (metadataAbort.signal.aborted) return

            const eventStream = new EventSource(buildRunEventsUrl(selectedRunId))
            eventStream.onmessage = handleMessage
            eventSource = eventStream
        }

        startScopedStream().catch(() => null)

        return () => {
            source.close()
        }
    }, [selectedRunId, addLog, setNodeStatus, clearHumanGate, resetNodeStatuses, setHumanGate, setRuntimeStatus, setRuntimeOutcome, setSelectedRunId])

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
                    {runtimeApiDegradedMessage ? (
                        <p data-testid="runtime-api-degraded-banner" className="mt-1 text-[10px] font-normal leading-4 text-amber-800">
                            {runtimeApiDegradedMessage}
                        </p>
                    ) : null}
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
