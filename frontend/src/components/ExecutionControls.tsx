import { useEffect, useMemo, useState } from 'react'
import { OctagonX, Play } from 'lucide-react'
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
    success: 'Complete',
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
    success: 'Cancel',
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
    'success',
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
    const cancelActionLabel = CANCEL_ACTION_LABELS[runtimeStatus] || 'Cancel'
    const transitionHint = TRANSITION_HINTS[runtimeStatus] || null
    const cancelDisabledReason = !selectedRunId
        ? 'Run id is still loading.'
        : CANCEL_DISABLED_REASONS[runtimeStatus] || transitionHint || DEFAULT_CANCEL_DISABLED_REASON
    const showRunStatusRow = runIsActive || Boolean(selectedRunId) || Boolean(humanGate)
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
        try {
            await fetchPipelineCancelValidated(selectedRunId)
        } catch (error) {
            logUnexpectedExecutionError(error)
            setRuntimeStatus('running')
            window.alert('Failed to request cancel. Check backend logs for details.')
        }
    }

    return (
        <div
            data-testid="execution-footer-controls"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
            className={`absolute bottom-4 z-20 rounded-md border border-border bg-background/95 shadow-lg backdrop-blur ${isNarrowViewport
                ? 'left-2 right-2 px-3 py-3'
                : 'left-1/2 w-[min(92vw,1040px)] -translate-x-1/2 px-4 py-3'
                }`}
        >
            <div className={`flex ${isNarrowViewport ? 'flex-col items-stretch gap-2' : 'flex-wrap items-center gap-2'}`}>
                {parsedLaunchInputs.entries.length > 0 ? (
                    <div
                        data-testid="execution-launch-inputs"
                        className="w-full rounded-md border border-border/80 bg-muted/20 px-3 py-3"
                    >
                        <div className="mb-3 space-y-1">
                            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                Launch Inputs
                            </p>
                            <p className="text-[11px] text-muted-foreground">
                                These values populate `context.*` at run start.
                            </p>
                            {parsedLaunchInputs.error ? (
                                <p className="text-[11px] text-destructive">{parsedLaunchInputs.error}</p>
                            ) : null}
                        </div>
                        <div className="grid gap-3">
                            {parsedLaunchInputs.entries.map((entry) => (
                                <div key={entry.key} className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">
                                        {entry.label}
                                        {entry.required ? ' *' : ''}
                                    </label>
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
                                            rows={3}
                                            className="min-h-20 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
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
                                            rows={4}
                                            className="min-h-20 w-full rounded-md border border-input bg-background px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            placeholder='{"key":"value"}'
                                        />
                                    )}
                                    {entry.description ? (
                                        <p className="text-[11px] text-muted-foreground">{entry.description}</p>
                                    ) : null}
                                </div>
                            ))}
                        </div>
                    </div>
                ) : null}
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
                <button
                    data-testid="execute-button"
                    onClick={() => {
                        void requestStart()
                    }}
                    disabled={!activeProjectPath || !executionFlowName || hasValidationErrors}
                    title={executeDisabledReason}
                    className="inline-flex h-9 items-center justify-center gap-2 whitespace-nowrap rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                >
                    <Play className="h-4 w-4" />
                    Execute
                </button>
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
                        {terminalStateLabel && (
                            <span data-testid="execution-footer-terminal-state" className="text-xs font-medium text-muted-foreground">
                                {terminalStateLabel}
                            </span>
                        )}
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
    )
}
