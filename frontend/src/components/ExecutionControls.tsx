import { useEffect, useMemo, useState } from 'react'
import { ChevronDown, ChevronUp, OctagonX, Play } from 'lucide-react'
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

const launchInputTypeLabel = (type: string) => {
    switch (type) {
        case 'string':
            return 'Text'
        case 'string[]':
            return 'List'
        case 'number':
            return 'Number'
        case 'boolean':
            return 'Boolean'
        default:
            return 'JSON'
    }
}

const launchInputDesktopSpanClass = (
    type: string,
    index: number,
    totalEntries: number,
    required: boolean,
) => {
    if (totalEntries === 1) {
        return 'lg:col-span-12'
    }
    if (type === 'boolean' || type === 'number') {
        return 'lg:col-span-4'
    }
    if (index === 0 && required && type === 'string' && totalEntries <= 3) {
        return 'lg:col-span-12'
    }
    return 'lg:col-span-6'
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
            <div
                data-testid="execution-canvas-primary-action"
                className={`absolute z-20 ${isNarrowViewport ? 'top-2 right-2' : 'top-4 right-4'}`}
            >
                <button
                    data-testid="execute-button"
                    onClick={() => {
                        void requestStart()
                    }}
                    disabled={!activeProjectPath || !executionFlowName || hasValidationErrors}
                    title={executeDisabledReason}
                    className="inline-flex h-9 items-center justify-center gap-2 whitespace-nowrap rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground shadow-lg transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                >
                    <Play className="h-4 w-4" />
                    Execute
                </button>
            </div>
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
                <div
                    data-testid="execution-launch-inputs"
                    className="mb-3 w-full"
                >
                    <div
                        data-testid="execution-launch-inputs-toolbar"
                        className="mb-2 flex items-center justify-between gap-3"
                    >
                        <p
                            data-testid="execution-launch-inputs-title"
                            className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                        >
                            Launch Inputs
                        </p>
                        {canCollapseLaunchInputs ? (
                            <button
                                type="button"
                                data-testid="execution-launch-inputs-toggle"
                                aria-label={launchInputsCollapsed ? 'Expand launch inputs' : 'Collapse launch inputs'}
                                title={launchInputsCollapsed ? 'Expand launch inputs' : 'Collapse launch inputs'}
                                onClick={() => {
                                    if (!executionFlowName) {
                                        return
                                    }
                                    setCollapsedLaunchInputsByFlow((current) => ({
                                        ...current,
                                        [executionFlowName]: !launchInputsCollapsed,
                                    }))
                                }}
                                className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            >
                                {launchInputsCollapsed ? (
                                    <ChevronUp className="h-3.5 w-3.5" />
                                ) : (
                                    <ChevronDown className="h-3.5 w-3.5" />
                                )}
                            </button>
                        ) : null}
                    </div>
                    {parsedLaunchInputs.error ? (
                        <p
                            data-testid="execution-launch-inputs-schema-error"
                            className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-[11px] text-destructive"
                        >
                            {parsedLaunchInputs.error}
                        </p>
                    ) : null}
                    {!launchInputsCollapsed ? (
                        <div
                            data-testid="execution-launch-inputs-body"
                            className="max-h-[min(42vh,20rem)] overflow-y-auto overscroll-contain"
                        >
                            <div
                                data-testid="execution-launch-inputs-grid"
                                className={`grid gap-x-4 gap-y-3 ${isNarrowViewport ? 'grid-cols-1' : 'grid-cols-1 lg:grid-cols-12'}`}
                            >
                                {parsedLaunchInputs.entries.map((entry, index) => (
                                    <div
                                        key={entry.key}
                                        data-testid={`execution-launch-input-field-${entry.key}`}
                                        className={`space-y-1.5 ${
                                            isNarrowViewport
                                                ? 'col-span-1'
                                                : launchInputDesktopSpanClass(
                                                    entry.type,
                                                    index,
                                                    launchInputCount,
                                                    entry.required,
                                                )
                                        }`}
                                    >
                                        <div
                                            className={`border-b border-border/40 pb-1 ${
                                                isNarrowViewport ? 'space-y-1' : 'flex items-start justify-between gap-3'
                                            }`}
                                        >
                                            <div className="min-w-0">
                                                <label className="text-xs font-medium text-foreground">
                                                    {entry.label}
                                                </label>
                                                {entry.description ? (
                                                    <p className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
                                                        {entry.description}
                                                    </p>
                                                ) : null}
                                            </div>
                                            <p className="shrink-0 text-[10px] leading-4 text-muted-foreground">
                                                {launchInputTypeLabel(entry.type)}
                                                {entry.required ? ' · Required' : ''}
                                            </p>
                                        </div>
                                        {entry.type === 'string' ? (
                                            <input
                                                data-testid={`execution-launch-input-${entry.key}`}
                                                value={launchInputValues[entry.key] ?? ''}
                                                onChange={(event) => setLaunchInputValues((current) => ({
                                                    ...current,
                                                    [entry.key]: event.target.value,
                                                }))}
                                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            />
                                        ) : entry.type === 'string[]' ? (
                                            <textarea
                                                data-testid={`execution-launch-input-${entry.key}`}
                                                value={launchInputValues[entry.key] ?? ''}
                                                onChange={(event) => setLaunchInputValues((current) => ({
                                                    ...current,
                                                    [entry.key]: event.target.value,
                                                }))}
                                                rows={2}
                                                className="min-h-16 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="One item per line"
                                            />
                                        ) : entry.type === 'boolean' ? (
                                            <select
                                                data-testid={`execution-launch-input-${entry.key}`}
                                                value={launchInputValues[entry.key] ?? ''}
                                                onChange={(event) => setLaunchInputValues((current) => ({
                                                    ...current,
                                                    [entry.key]: event.target.value,
                                                }))}
                                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            >
                                                <option value="">Unset</option>
                                                <option value="true">True</option>
                                                <option value="false">False</option>
                                            </select>
                                        ) : entry.type === 'number' ? (
                                            <input
                                                data-testid={`execution-launch-input-${entry.key}`}
                                                type="number"
                                                value={launchInputValues[entry.key] ?? ''}
                                                onChange={(event) => setLaunchInputValues((current) => ({
                                                    ...current,
                                                    [entry.key]: event.target.value,
                                                }))}
                                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="42"
                                            />
                                        ) : (
                                            <textarea
                                                data-testid={`execution-launch-input-${entry.key}`}
                                                value={launchInputValues[entry.key] ?? ''}
                                                onChange={(event) => setLaunchInputValues((current) => ({
                                                    ...current,
                                                    [entry.key]: event.target.value,
                                                }))}
                                                rows={3}
                                                className="min-h-20 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder='{"key":"value"}'
                                            />
                                        )}
                                    </div>
                                ))}
                            </div>
                        </div>
                    ) : null}
                </div>
            ) : null}
            <div className={`flex ${isNarrowViewport ? 'flex-col items-stretch gap-2' : 'flex-wrap items-center gap-2'}`}>
                {showValidationWarningBanner ? (
                    <p
                        data-testid="execute-warning-banner"
                        className="rounded border border-amber-400 bg-amber-50 px-2 py-1 text-[11px] font-medium leading-none text-amber-900"
                    >
                        Warnings present; run allowed.
                    </p>
                ) : null}
                {runStartGitPolicyWarning ? (
                    <p
                        data-testid="run-start-git-policy-warning-banner"
                        className="max-w-sm truncate rounded border border-amber-400 bg-amber-50 px-2 py-1 text-[11px] font-medium leading-none text-amber-900"
                    >
                        {runStartGitPolicyWarning}
                    </p>
                ) : null}
                {runStartError ? (
                    <p
                        data-testid="run-start-error-banner"
                        className="max-w-sm truncate rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] font-medium leading-none text-destructive"
                    >
                        Failed to start run: {runStartError}
                    </p>
                ) : null}
                {lastLaunchFailure ? (
                    <div
                        data-testid="launch-failure-diagnostics"
                        className="max-w-sm rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive"
                    >
                        <p className="font-medium">Last launch failure</p>
                        <p data-testid="launch-failure-message" className="truncate">
                            {lastLaunchFailure.message}
                        </p>
                        <p className="truncate">
                            Flow source: <span className="font-mono">{lastLaunchFailure.flowSource || 'none'}</span>
                        </p>
                        <p>Failed at: {new Date(lastLaunchFailure.failedAt).toLocaleString()}</p>
                        <button
                            data-testid="launch-retry-button"
                            onClick={() => {
                                void requestStart()
                            }}
                            disabled={!canRetryLaunch}
                            className="mt-1 rounded border border-destructive/40 bg-background px-2 py-1 text-[11px] font-medium text-destructive hover:bg-destructive/5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                        >
                            Retry launch
                        </button>
                        {!canRetryLaunch ? (
                            <p data-testid="launch-retry-disabled-reason" className="mt-1">
                                Resolve launch blockers to retry.
                            </p>
                        ) : null}
                    </div>
                ) : null}
            </div>
            {showRunStatusRow ? (
                <>
                    <div className="my-3 h-px bg-border" />
                    <div className={`flex ${isNarrowViewport ? 'flex-col items-stretch gap-2' : 'flex-wrap items-center gap-3'}`}>
                        {humanGate && (
                            <div
                                data-testid="execution-pending-human-gate-banner"
                                className="inline-flex items-center rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-[11px] font-semibold text-amber-800"
                            >
                                Pending human gate: {humanGate.prompt || humanGate.nodeId}
                            </div>
                        )}
                        <span data-testid="execution-footer-run-status" className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                            {statusLabel}
                        </span>
                        <span data-testid="execution-footer-run-identity" className="text-xs font-mono text-muted-foreground">
                            {runIdentityLabel}
                        </span>
                        {runtimeOutcome ? (
                            <span data-testid="execution-footer-run-outcome" className="text-xs font-medium text-muted-foreground">
                                Outcome: {outcomeLabel}
                            </span>
                        ) : null}
                        {terminalStateLabel && (
                            <span data-testid="execution-footer-terminal-state" className="text-xs font-medium text-muted-foreground">
                                {terminalStateLabel}
                            </span>
                        )}
                        {runtimeOutcomeReasonCode ? (
                            <span data-testid="execution-footer-outcome-reason-code" className="text-xs text-muted-foreground">
                                Reason: {runtimeOutcomeReasonCode}
                            </span>
                        ) : null}
                        {runtimeOutcomeReasonMessage ? (
                            <span data-testid="execution-footer-outcome-reason-message" className="text-xs text-muted-foreground">
                                {runtimeOutcomeReasonMessage}
                            </span>
                        ) : null}
                        {transitionHint && (
                            <span className="text-xs text-muted-foreground">{transitionHint}</span>
                        )}
                        <button
                            data-testid="execution-footer-cancel-button"
                            onClick={requestCancel}
                            disabled={!canCancel}
                            title={canCancel ? undefined : cancelDisabledReason}
                            className="inline-flex h-8 items-center gap-2 rounded-md bg-destructive px-2 text-xs font-semibold uppercase tracking-wide text-destructive-foreground transition-colors hover:bg-destructive/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                        >
                            <OctagonX className="h-3.5 w-3.5" />
                            {cancelActionLabel}
                        </button>
                        <button
                            data-testid="execution-footer-pause-button"
                            disabled={true}
                            title={UNSUPPORTED_CONTROL_REASON}
                            className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                        >
                            Pause
                        </button>
                        <button
                            data-testid="execution-footer-resume-button"
                            disabled={true}
                            title={UNSUPPORTED_CONTROL_REASON}
                            className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                        >
                            Resume
                        </button>
                        <span
                            data-testid="execution-footer-unsupported-controls-reason"
                            className={`text-xs text-muted-foreground ${isNarrowViewport ? 'max-w-none' : 'max-w-xs'}`}
                        >
                            {UNSUPPORTED_CONTROL_REASON}
                        </span>
                    </div>
                </>
            ) : null}
                </div>
            ) : null}
        </>
    )
}
