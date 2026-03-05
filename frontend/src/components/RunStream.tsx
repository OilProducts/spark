import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { fetchPipelineStatusValidated, fetchRuntimeStatusValidated } from '@/lib/apiClient'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { resolveSelectedRunScopePreflight, resolveStatusHydrationDecision } from '@/lib/runScope'

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
const RUNTIME_STATUS_DEGRADED_MESSAGE =
    'Runtime status endpoint is unavailable or incompatible. Live runtime hydration is in degraded mode.'
const SELECTED_RUN_STATUS_DEGRADED_MESSAGE =
    'Selected run status endpoint is unavailable or incompatible. Live event streaming preflight is in degraded mode.'

interface RuntimeStageCursor {
    stageIndex: number
    status: 'running' | 'success' | 'failed'
    pendingRetry: boolean
}

const normalizeScopePath = (value: string) => {
    const trimmed = value.trim()
    if (!trimmed) return ''
    const slashNormalized = trimmed.replace(/\\/g, '/').replace(/\/{2,}/g, '/')
    const prefix = slashNormalized.startsWith('/') ? '/' : ''
    const rawBody = prefix ? slashNormalized.slice(1) : slashNormalized
    const parts = rawBody.split('/').filter((part) => part.length > 0)
    const segments: string[] = []
    for (const part of parts) {
        if (part === '.') {
            continue
        }
        if (part === '..') {
            segments.pop()
            continue
        }
        segments.push(part)
    }
    const normalizedBody = segments.join('/')
    if (!normalizedBody && prefix) {
        return prefix
    }
    return `${prefix}${normalizedBody}`
}

const runBelongsToProjectScope = (runWorkingDirectory: string, projectPath: string | null) => {
    if (!projectPath) return false
    const normalizedProjectPath = normalizeScopePath(projectPath)
    if (!normalizedProjectPath) return false

    const normalizedRunWorkingDirectory = normalizeScopePath(runWorkingDirectory)
    if (!normalizedRunWorkingDirectory) return false
    if (normalizedRunWorkingDirectory === normalizedProjectPath) return true
    return normalizedRunWorkingDirectory.startsWith(`${normalizedProjectPath}/`)
}

export function RunStream() {
    const addLog = useStore((state) => state.addLog)
    const clearLogs = useStore((state) => state.clearLogs)
    const setNodeStatus = useStore((state) => state.setNodeStatus)
    const setHumanGate = useStore((state) => state.setHumanGate)
    const clearHumanGate = useStore((state) => state.clearHumanGate)
    const resetNodeStatuses = useStore((state) => state.resetNodeStatuses)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const saveState = useStore((state) => state.saveState)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const [runtimeApiDegradedMessage, setRuntimeApiDegradedMessage] = useState<string | null>(null)
    const stageCursorsRef = useRef<Record<string, RuntimeStageCursor>>({})
    const saveStateLabel =
        saveState === 'saving'
            ? 'Saving...'
            : saveState === 'saved'
                ? 'Saved'
                : saveState === 'conflict'
                    ? 'Save Conflict'
                : saveState === 'error'
                    ? 'Save Failed'
                    : 'Idle'
    const remediation = resolveSaveRemediation(saveState, saveErrorKind)

    useEffect(() => {
        stageCursorsRef.current = {}
        resetNodeStatuses()
        clearHumanGate()
        clearLogs()
        setRuntimeStatus('idle')
    }, [selectedRunId, resetNodeStatuses, clearHumanGate, clearLogs, setRuntimeStatus])

    useEffect(() => {
        let isCancelled = false
        fetchRuntimeStatusValidated()
            .then((data) => {
                if (isCancelled) return
                setRuntimeApiDegradedMessage((current) =>
                    current === RUNTIME_STATUS_DEGRADED_MESSAGE ? null : current
                )
                const runId = typeof data?.last_run_id === 'string' ? data.last_run_id : null
                const lastWorkingDirectory = typeof data?.last_working_directory === 'string' ? data.last_working_directory : ''
                const statusRunInScope = runBelongsToProjectScope(lastWorkingDirectory, activeProjectPath)
                const statusHydrationDecision = resolveStatusHydrationDecision({
                    selectedRunId,
                    statusRunId: runId,
                    statusRunWorkingDirectory: lastWorkingDirectory,
                    activeProjectPath,
                    statusRuntimeStatus: typeof data?.status === 'string' ? data.status : null,
                })
                if (!selectedRunId && runId && statusRunInScope) {
                    if (statusHydrationDecision.nextSelectedRunId) {
                        setSelectedRunId(statusHydrationDecision.nextSelectedRunId)
                    }
                }
                if (!selectedRunId && (!runId || !statusRunInScope)) {
                    setRuntimeStatus('idle')
                    return
                }
                if (data?.status && ((!selectedRunId && statusRunInScope) || runId === selectedRunId)) {
                    if (statusHydrationDecision.nextRuntimeStatus) {
                        setRuntimeStatus(statusHydrationDecision.nextRuntimeStatus)
                    }
                }
            })
            .catch(() => {
                if (isCancelled) return
                setRuntimeApiDegradedMessage(RUNTIME_STATUS_DEGRADED_MESSAGE)
            })
        return () => {
            isCancelled = true
        }
    }, [selectedRunId, activeProjectPath, setRuntimeStatus, setSelectedRunId])

    useEffect(() => {
        if (!selectedRunId) {
            setRuntimeApiDegradedMessage((current) =>
                current === SELECTED_RUN_STATUS_DEGRADED_MESSAGE ? null : current
            )
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
                        if (runtimeNodeStatus !== 'waiting' && currentGate?.nodeId === runtimeNodeId) {
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
                }
                if (data.type === 'runtime' && data.status) {
                    setRuntimeStatus(data.status)
                }
            } catch {
                // ignore malformed events
            }
        }

        const startScopedStream = async () => {
            let data: Awaited<ReturnType<typeof fetchPipelineStatusValidated>> | null = null
            if (!metadataAbort.signal.aborted) {
                try {
                    data = await fetchPipelineStatusValidated(selectedRunId)
                    setRuntimeApiDegradedMessage((current) =>
                        current === SELECTED_RUN_STATUS_DEGRADED_MESSAGE ? null : current
                    )
                } catch {
                    if (!metadataAbort.signal.aborted) {
                        setRuntimeApiDegradedMessage(SELECTED_RUN_STATUS_DEGRADED_MESSAGE)
                    }
                    return
                }
            }
            if (metadataAbort.signal.aborted) return

            const selectedRunWorkingDirectory = typeof data?.working_directory === 'string' ? data.working_directory : ''
            const selectedRunInScope = runBelongsToProjectScope(selectedRunWorkingDirectory, activeProjectPath)
            const preflightDecision = resolveSelectedRunScopePreflight({
                selectedRunWorkingDirectory,
                activeProjectPath,
                selectedRunStatus: typeof data?.status === 'string' ? data.status : null,
            })
            if (!selectedRunInScope) {
                if (preflightDecision.clearSelectedRun) {
                    setSelectedRunId(null)
                }
                if (preflightDecision.nextRuntimeStatus) {
                    setRuntimeStatus(preflightDecision.nextRuntimeStatus)
                }
                return
            }
            if (preflightDecision.nextRuntimeStatus) {
                setRuntimeStatus(preflightDecision.nextRuntimeStatus)
            }
            if (metadataAbort.signal.aborted) return

            const source = new EventSource(`/pipelines/${encodeURIComponent(selectedRunId)}/events`)
            source.onmessage = handleMessage
            eventSource = source
        }

        startScopedStream().catch(() => null)

        return () => {
            source.close()
        }
    }, [selectedRunId, activeProjectPath, addLog, setNodeStatus, clearHumanGate, resetNodeStatuses, setHumanGate, setRuntimeStatus, setSelectedRunId])

    return (
        <div data-testid="execution-runtime-stream-indicator" className="pointer-events-none fixed right-4 top-16 z-[70]">
            <div
                data-testid="global-save-state-indicator"
                className={`rounded-md border px-2 py-1 text-[11px] font-medium shadow-sm ${
                    saveState === 'error'
                        ? 'border-destructive/50 bg-destructive/10 text-destructive'
                        : saveState === 'conflict'
                            ? 'border-amber-500/50 bg-amber-500/10 text-amber-800'
                        : saveState === 'saved'
                            ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-700'
                            : 'border-border bg-background/95 text-muted-foreground'
                }`}
                title={saveErrorMessage || undefined}
            >
                <span>{saveStateLabel}</span>
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
            </div>
        </div>
    )
}
