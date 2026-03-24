import { useEffect, useMemo, useState } from 'react'
import { useStore, type RuntimeStatus } from '@/store'
import { buildPipelineStartPayload, type PipelineStartPayload } from '@/lib/pipelineStartPayload'
import {
    ApiHttpError,
    fetchFlowPayloadValidated,
    fetchPipelineCancelValidated,
    fetchPipelineStartValidated,
} from '@/lib/attractorClient'
import { fetchProjectMetadataValidated } from '@/lib/workspaceClient'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import {
    buildLaunchContextFromValues,
    initializeLaunchInputFormValues,
    parseLaunchInputDefinitions,
    type LaunchInputFormValues,
} from '@/lib/flowContracts'
import { Separator } from '@/ui'
import { ExecutionActionOverlay } from './components/ExecutionActionOverlay'
import { ExecutionLaunchInputsSurface } from './components/ExecutionLaunchInputsSurface'
import { ExecutionNoticeStack } from './components/ExecutionNoticeStack'
import { ExecutionRunStatusStrip } from './components/ExecutionRunStatusStrip'

type LaunchFailureDiagnostics = {
    message: string
    failedAt: string
    flowSource: string | null
}

const STATUS_LABELS: Record<string, string> = {
    running: 'Running',
    abort_requested: 'Canceling…',
    cancel_requested: 'Canceling…',
    aborted: 'Canceled',
    canceled: 'Canceled',
    failed: 'Failed',
    validation_error: 'Validation Error',
    completed: 'Completed',
    idle: 'Idle',
}

const CANCEL_ACTION_LABELS: Record<string, string> = {
    running: 'Cancel',
    abort_requested: 'Canceling…',
    cancel_requested: 'Canceling…',
    aborted: 'Canceled',
    canceled: 'Canceled',
    failed: 'Cancel',
    validation_error: 'Cancel',
    completed: 'Cancel',
    idle: 'Cancel',
}

const TRANSITION_HINTS: Record<string, string> = {
    abort_requested: 'Cancel requested. Waiting for active node to finish.',
    cancel_requested: 'Cancel requested. Waiting for active node to finish.',
    aborted: 'Run canceled.',
    canceled: 'Run canceled.',
}

const CANCEL_DISABLED_REASONS: Record<string, string> = {
    cancel_requested: 'Cancel already requested for this run.',
    abort_requested: 'Cancel already requested for this run.',
    canceled: 'This run is already canceled.',
    aborted: 'This run is already canceled.',
}

const DEFAULT_CANCEL_DISABLED_REASON = 'Cancel is only available while the run is active.'

const UNSUPPORTED_CONTROL_REASON = 'Pause/Resume is unavailable: backend runtime control API does not expose pause/resume.'
const ACTIVE_RUNTIME_STATUSES = new Set<RuntimeStatus>([
    'running',
    'cancel_requested',
    'abort_requested',
])
const TERMINAL_RUNTIME_STATUSES = new Set<RuntimeStatus>([
    'completed',
    'failed',
    'validation_error',
    'canceled',
    'aborted',
])

const logUnexpectedExecutionError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

export function ExecutionControls() {
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const activeProjectScope = useStore((state) =>
        state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath] : null
    )
    const executionFlow = useStore((state) => state.executionFlow)
    const workingDir = useStore((state) => state.workingDir)
    const model = useStore((state) => state.model)
    const graphAttrs = useStore((state) => state.executionGraphAttrs)
    const diagnostics = useStore((state) => state.executionDiagnostics)
    const hasValidationErrors = useStore((state) => state.executionHasValidationErrors)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const runtimeOutcome = useStore((state) => state.runtimeOutcome)
    const runtimeOutcomeReasonCode = useStore((state) => state.runtimeOutcomeReasonCode)
    const runtimeOutcomeReasonMessage = useStore((state) => state.runtimeOutcomeReasonMessage)
    const setRuntimeOutcome = useStore((state) => state.setRuntimeOutcome)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const humanGate = useStore((state) => state.humanGate)
    const isNarrowViewport = useNarrowViewport()
    const hasValidationWarnings = diagnostics.some((diag) => diag.severity === 'warning')
    const showValidationWarningBanner = hasValidationWarnings && !hasValidationErrors
    const [runStartError, setRunStartError] = useState<string | null>(null)
    const [lastLaunchFailure, setLastLaunchFailure] = useState<LaunchFailureDiagnostics | null>(null)
    const [runStartGitPolicyWarning, setRunStartGitPolicyWarning] = useState<string | null>(null)
    const [launchInputValues, setLaunchInputValues] = useState<LaunchInputFormValues>({})
    const [collapsedLaunchInputsByFlow, setCollapsedLaunchInputsByFlow] = useState<Record<string, boolean>>({})
    const executionFlowName = executionFlow
    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(graphAttrs['spark.launch_inputs']),
        [graphAttrs],
    )
    const runInitiationForm = {
        projectPath: activeProjectPath || '',
        flowSource: executionFlowName || '',
        workingDirectory: workingDir,
        backend: 'codex-app-server',
        model: model.trim() || null,
        launchContext: null,
        specArtifactId: activeProjectScope?.specId || null,
        planArtifactId: activeProjectScope?.planId || null,
    }
    const canRetryLaunch = Boolean(activeProjectPath) && Boolean(executionFlowName) && !hasValidationErrors
    const launchInputCount = parsedLaunchInputs.entries.length

    const runIsActive = ACTIVE_RUNTIME_STATUSES.has(runtimeStatus)
    const shouldShowFooter = viewMode === 'execution'
    const canCancel = runtimeStatus === 'running' && Boolean(selectedRunId)
    const statusLabel = useMemo(
        () => STATUS_LABELS[runtimeStatus] || runtimeStatus,
        [runtimeStatus]
    )
    const runIdentityLabel = selectedRunId ? `Run ${selectedRunId}` : 'Run id loading…'
    const isTerminalState = TERMINAL_RUNTIME_STATUSES.has(runtimeStatus)
    const terminalStateLabel = isTerminalState ? `Terminal: ${statusLabel}` : null
    const outcomeLabel = runtimeOutcome === 'success' ? 'Success' : runtimeOutcome === 'failure' ? 'Failure' : '—'
    const cancelActionLabel = CANCEL_ACTION_LABELS[runtimeStatus] || 'Cancel'
    const transitionHint = TRANSITION_HINTS[runtimeStatus] || null
    const cancelDisabledReason = !selectedRunId
        ? 'Run id is still loading.'
        : CANCEL_DISABLED_REASONS[runtimeStatus] || transitionHint || DEFAULT_CANCEL_DISABLED_REASON
    const showRunStatusRow = runIsActive || Boolean(selectedRunId) || Boolean(humanGate)
    const showLaunchInputs = parsedLaunchInputs.entries.length > 0 || Boolean(parsedLaunchInputs.error)
    const canCollapseLaunchInputs = parsedLaunchInputs.entries.length > 0
    const launchInputsCollapsed = executionFlowName ? (collapsedLaunchInputsByFlow[executionFlowName] ?? false) : false
    const hasFooterAuxiliaryContent = (
        showValidationWarningBanner
        || Boolean(runStartGitPolicyWarning)
        || Boolean(runStartError)
        || Boolean(lastLaunchFailure)
        || showRunStatusRow
    )
    const showFooterSurface = (
        showLaunchInputs
        || hasFooterAuxiliaryContent
    )
    const footerDesktopWidthClass = showLaunchInputs && !hasFooterAuxiliaryContent
        ? 'w-[calc(100%-2rem)] max-w-3xl'
        : 'w-[calc(100%-2rem)] max-w-[960px]'
    const executeDisabledReason = !activeProjectPath
        ? 'Select an active project before running.'
        : !executionFlowName
            ? 'Select an active flow before running.'
            : hasValidationErrors
                ? 'Fix validation errors before running.'
                : showValidationWarningBanner
                    ? 'Warnings are present. Review diagnostics before running.'
                    : undefined

    useEffect(() => {
        setLaunchInputValues((current) => initializeLaunchInputFormValues(parsedLaunchInputs.entries, current))
    }, [parsedLaunchInputs.entries])

    useEffect(() => {
        setRunStartError(null)
        setLastLaunchFailure(null)
        setRunStartGitPolicyWarning(null)
    }, [executionFlowName])

    if (!shouldShowFooter) return null

    const confirmGitPolicyGate = async () => {
        try {
            const metadata = await fetchProjectMetadataValidated(runInitiationForm.projectPath)
            const branch = typeof metadata.branch === 'string' ? metadata.branch.trim() : ''
            if (branch) {
                setRunStartGitPolicyWarning(null)
                return true
            }

            const warning = 'Project Git policy check failed: active project is not a Git repository.'
            setRunStartGitPolicyWarning(warning)
            const allowNonGitRun = window.confirm(`${warning} Continue with run start anyway?`)
            return allowNonGitRun
        } catch (err) {
            const warning = 'Unable to verify project Git state before run start.'
            if (err instanceof ApiHttpError && err.detail) {
                console.warn(err.detail)
            }
            setRunStartGitPolicyWarning(warning)
            return window.confirm(`${warning} Continue with run start anyway?`)
        }
    }

    const requestStart = async () => {
        if (!activeProjectPath || !executionFlowName || hasValidationErrors) return

        setRunStartError(null)
        if (parsedLaunchInputs.error) {
            setRunStartError(`Flow launch input schema is invalid: ${parsedLaunchInputs.error}`)
            return
        }
        const { launchContext, errors: launchContextErrors } = buildLaunchContextFromValues(
            parsedLaunchInputs.entries,
            launchInputValues,
        )
        if (launchContextErrors.length > 0) {
            setRunStartError(launchContextErrors.join(' '))
            return
        }
        try {
            const gitPolicyGateAllowed = await confirmGitPolicyGate()
            if (!gitPolicyGateAllowed) {
                return
            }

            const flow = await fetchFlowPayloadValidated(runInitiationForm.flowSource)
            const resolvedWorkingDirectory = runInitiationForm.workingDirectory.trim() || runInitiationForm.projectPath
            const startPayload = buildPipelineStartPayload(
                {
                    ...runInitiationForm,
                    launchContext,
                    workingDirectory: resolvedWorkingDirectory,
                },
                flow.content
            )
            const runData = await fetchPipelineStartValidated(startPayload as PipelineStartPayload)
            if (runData?.status !== 'started') {
                const reason = runData?.error || runData?.status || 'Unknown run error'
                throw new Error(`Run not started: ${reason}`)
            }
            if (typeof runData?.pipeline_id === 'string') {
                setSelectedRunId(runData.pipeline_id)
            }
            setRuntimeStatus('running')
            setRuntimeOutcome(null)

            setLastLaunchFailure(null)
        } catch (error) {
            logUnexpectedExecutionError(error)
            const errorMessage = error instanceof ApiHttpError && error.detail
                ? error.detail
                : error instanceof Error
                    ? error.message
                    : 'Failed to start pipeline run.'
            setRunStartError(errorMessage)
            setLastLaunchFailure({
                message: errorMessage,
                failedAt: new Date().toISOString(),
                flowSource: runInitiationForm.flowSource || null,
            })
        }
    }

    const requestCancel = async () => {
        if (!selectedRunId) {
            window.alert('Run id is still loading. Please try cancel again in a moment.')
            return
        }
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        setRuntimeStatus('cancel_requested')
        setRuntimeOutcome(null)
        try {
            await fetchPipelineCancelValidated(selectedRunId)
        } catch (error) {
            logUnexpectedExecutionError(error)
            setRuntimeStatus('running')
            window.alert('Failed to request cancel. Check backend logs for details.')
        }
    }

    return (
        <>
            <ExecutionActionOverlay
                isNarrowViewport={isNarrowViewport}
                disabled={!activeProjectPath || !executionFlowName || hasValidationErrors}
                disabledReason={executeDisabledReason}
                onExecute={() => {
                    void requestStart()
                }}
            />
            {showFooterSurface ? (
                <div
                    data-testid="execution-footer-controls"
                    data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
                    className={`absolute bottom-4 z-20 rounded-md border border-border bg-background/95 shadow-lg backdrop-blur ${isNarrowViewport
                        ? 'left-2 right-2 px-3 py-3'
                        : `left-1/2 ${footerDesktopWidthClass} -translate-x-1/2 px-4 py-3`
                        }`}
                >
                    {showLaunchInputs ? (
                        <ExecutionLaunchInputsSurface
                            isNarrowViewport={isNarrowViewport}
                            executionFlowName={executionFlowName}
                            parsedLaunchInputs={parsedLaunchInputs}
                            launchInputValues={launchInputValues}
                            launchInputCount={launchInputCount}
                            launchInputsCollapsed={launchInputsCollapsed}
                            canCollapseLaunchInputs={canCollapseLaunchInputs}
                            onToggleCollapsed={() => {
                                if (!executionFlowName) {
                                    return
                                }
                                setCollapsedLaunchInputsByFlow((current) => ({
                                    ...current,
                                    [executionFlowName]: !launchInputsCollapsed,
                                }))
                            }}
                            onInputChange={(entry, value) => {
                                setLaunchInputValues((current) => ({
                                    ...current,
                                    [entry.key]: value,
                                }))
                            }}
                        />
                    ) : null}
                    <ExecutionNoticeStack
                        showValidationWarningBanner={showValidationWarningBanner}
                        runStartGitPolicyWarning={runStartGitPolicyWarning}
                        runStartError={runStartError}
                        lastLaunchFailure={lastLaunchFailure}
                        canRetryLaunch={canRetryLaunch}
                        onRetry={() => {
                            void requestStart()
                        }}
                    />
            {showRunStatusRow ? (
                <>
                    <Separator className="my-3" />
                    <ExecutionRunStatusStrip
                        isNarrowViewport={isNarrowViewport}
                        humanGatePrompt={humanGate ? humanGate.prompt || humanGate.nodeId : null}
                        statusLabel={statusLabel}
                        runIdentityLabel={runIdentityLabel}
                        runtimeOutcome={runtimeOutcome}
                        outcomeLabel={outcomeLabel}
                        terminalStateLabel={terminalStateLabel}
                        runtimeOutcomeReasonCode={runtimeOutcomeReasonCode}
                        runtimeOutcomeReasonMessage={runtimeOutcomeReasonMessage}
                        transitionHint={transitionHint}
                        canCancel={canCancel}
                        cancelDisabledReason={cancelDisabledReason}
                        cancelActionLabel={cancelActionLabel}
                        unsupportedControlReason={UNSUPPORTED_CONTROL_REASON}
                        onCancel={requestCancel}
                    />
                </>
            ) : null}
                </div>
            ) : null}
        </>
    )
}
