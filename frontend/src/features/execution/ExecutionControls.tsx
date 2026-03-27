import { useEffect, useMemo, useState } from 'react'

import {
    buildLaunchContextFromValues,
    initializeLaunchInputFormValues,
    parseLaunchInputDefinitions,
    type LaunchInputFormValues,
} from '@/lib/flowContracts'
import { buildPipelineStartPayload, type PipelineStartPayload } from '@/lib/pipelineStartPayload'
import { formatProjectListLabel } from '@/features/projects/model/projectsHomeState'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useStore } from '@/store'
import { Button, Checkbox, InlineNotice, Label, Panel, PanelContent, PanelHeader, SectionHeader, useDialogController } from '@/ui'

import { useExecutionLaunchPreview } from './hooks/useExecutionLaunchPreview'
import { ApiHttpError, loadExecutionFlowPayload, loadExecutionProjectMetadata, startExecutionRun } from './services/executionRunService'
import { ExecutionLaunchInputsSurface } from './components/ExecutionLaunchInputsSurface'
import { ExecutionNoticeStack } from './components/ExecutionNoticeStack'

type LaunchFailureDiagnostics = {
    message: string
    failedAt: string
    flowSource: string | null
}

const logUnexpectedExecutionError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

export function ExecutionControls() {
    const { confirm } = useDialogController()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const activeProjectScope = useStore((state) =>
        state.activeProjectPath ? state.projectSessionsByPath[state.activeProjectPath] : null,
    )
    const executionFlow = useStore((state) => state.executionFlow)
    const workingDir = useStore((state) => state.workingDir)
    const model = useStore((state) => state.model)
    const graphAttrs = useStore((state) => state.executionGraphAttrs)
    const diagnostics = useStore((state) => state.executionDiagnostics)
    const hasValidationErrors = useStore((state) => state.executionHasValidationErrors)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const humanGate = useStore((state) => state.humanGate)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const setRuntimeOutcome = useStore((state) => state.setRuntimeOutcome)
    const setViewMode = useStore((state) => state.setViewMode)
    const isNarrowViewport = useNarrowViewport()

    const [runStartError, setRunStartError] = useState<string | null>(null)
    const [lastLaunchFailure, setLastLaunchFailure] = useState<LaunchFailureDiagnostics | null>(null)
    const [runStartGitPolicyWarning, setRunStartGitPolicyWarning] = useState<string | null>(null)
    const [launchInputValues, setLaunchInputValues] = useState<LaunchInputFormValues>({})
    const [collapsedLaunchInputsByFlow, setCollapsedLaunchInputsByFlow] = useState<Record<string, boolean>>({})
    const [openRunsAfterLaunch, setOpenRunsAfterLaunch] = useState(false)
    const [launchSuccessRunId, setLaunchSuccessRunId] = useState<string | null>(null)

    const executionFlowName = executionFlow
    const { isLoadingPreview, previewLoadError } = useExecutionLaunchPreview(executionFlowName)
    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(graphAttrs['spark.launch_inputs']),
        [graphAttrs],
    )
    const launchInputCount = parsedLaunchInputs.entries.length
    const launchInputsCollapsed = executionFlowName ? (collapsedLaunchInputsByFlow[executionFlowName] ?? false) : false
    const canCollapseLaunchInputs = parsedLaunchInputs.entries.length > 0
    const showValidationWarningBanner = diagnostics.some((diag) => diag.severity === 'warning') && !hasValidationErrors
    const executeLabel = activeProjectPath
        ? `Run in ${formatProjectListLabel(activeProjectPath)}`
        : 'Run'
    const executeDisabledReason = !activeProjectPath
        ? 'Select an active project before running.'
        : !executionFlowName
            ? 'Select an active flow before running.'
            : isLoadingPreview
                ? 'Loading flow preview for launch inputs.'
                : hasValidationErrors
                    ? 'Fix validation errors before running.'
                    : parsedLaunchInputs.error
                        ? 'Fix launch-input schema errors before running.'
                        : undefined
    const canRun = !executeDisabledReason
    const canRetryLaunch = Boolean(activeProjectPath) && Boolean(executionFlowName) && !hasValidationErrors
    const visibleDiagnostics = diagnostics.slice(0, 8)
    const pendingHumanGatePrompt = humanGate && humanGate.runId === selectedRunId ? humanGate.prompt : null
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

    useEffect(() => {
        setLaunchInputValues((current) => initializeLaunchInputFormValues(parsedLaunchInputs.entries, current))
    }, [parsedLaunchInputs.entries])

    useEffect(() => {
        setRunStartError(null)
        setLastLaunchFailure(null)
        setRunStartGitPolicyWarning(null)
        setLaunchSuccessRunId(null)
    }, [executionFlowName])

    const confirmGitPolicyGate = async () => {
        try {
            const metadata = await loadExecutionProjectMetadata(runInitiationForm.projectPath)
            const branch = typeof metadata.branch === 'string' ? metadata.branch.trim() : ''
            if (branch) {
                setRunStartGitPolicyWarning(null)
                return true
            }

            const warning = 'Project Git policy check failed: active project is not a Git repository.'
            setRunStartGitPolicyWarning(warning)
            return confirm({
                title: 'Run without Git metadata?',
                description: `${warning} Continue with run start anyway?`,
                confirmLabel: 'Continue',
                cancelLabel: 'Cancel',
            })
        } catch (err) {
            const warning = 'Unable to verify project Git state before run start.'
            if (err instanceof ApiHttpError && err.detail) {
                console.warn(err.detail)
            }
            setRunStartGitPolicyWarning(warning)
            return confirm({
                title: 'Unable to verify Git state',
                description: `${warning} Continue with run start anyway?`,
                confirmLabel: 'Continue',
                cancelLabel: 'Cancel',
            })
        }
    }

    const requestStart = async () => {
        if (!canRun || !activeProjectPath || !executionFlowName) {
            return
        }

        setRunStartError(null)
        setLaunchSuccessRunId(null)
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

            const flow = await loadExecutionFlowPayload(runInitiationForm.flowSource)
            const resolvedWorkingDirectory = runInitiationForm.workingDirectory.trim() || runInitiationForm.projectPath
            const startPayload = buildPipelineStartPayload(
                {
                    ...runInitiationForm,
                    launchContext,
                    workingDirectory: resolvedWorkingDirectory,
                },
                flow.content,
            )
            const runData = await startExecutionRun(startPayload as PipelineStartPayload)
            if (runData?.status !== 'started') {
                const reason = runData?.error || runData?.status || 'Unknown run error'
                throw new Error(`Run not started: ${reason}`)
            }

            const nextRunId = typeof runData?.pipeline_id === 'string' ? runData.pipeline_id : null
            if (nextRunId) {
                setSelectedRunId(nextRunId)
                setLaunchSuccessRunId(nextRunId)
            }
            setRuntimeStatus('running')
            setRuntimeOutcome(null)
            setLastLaunchFailure(null)

            if (openRunsAfterLaunch && nextRunId) {
                setViewMode('runs')
            }
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

    return (
        <div data-testid="execution-launch-panel" className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
            <Panel className="m-4 flex min-h-0 flex-1 flex-col overflow-hidden">
                <PanelHeader>
                    <SectionHeader
                        title="Launch Flow"
                        description={executionFlowName
                            ? `Direct-run launch inputs for ${executionFlowName}.`
                            : 'Select a flow to configure direct-run launch inputs.'}
                        action={executionFlowName ? (
                            <span
                                data-testid="execution-launch-flow-name"
                                className="max-w-[20rem] truncate font-mono text-xs text-muted-foreground"
                                title={executionFlowName}
                            >
                                {executionFlowName}
                            </span>
                        ) : undefined}
                    />
                </PanelHeader>
                <PanelContent className="flex min-h-0 flex-1 flex-col overflow-hidden">
                    {pendingHumanGatePrompt ? (
                        <div className="px-4 pb-1">
                            <InlineNotice data-testid="execution-pending-human-gate-banner" tone="warning">
                                <div className={`flex ${isNarrowViewport ? 'flex-col items-start gap-2' : 'items-center justify-between gap-3'}`}>
                                    <div>
                                        Pending human gate: <span className="font-medium">{pendingHumanGatePrompt}</span>
                                    </div>
                                    {selectedRunId ? (
                                        <Button
                                            type="button"
                                            variant="outline"
                                            size="xs"
                                            data-testid="execution-pending-human-gate-view-run-button"
                                            onClick={() => {
                                                setSelectedRunId(selectedRunId)
                                                setViewMode('runs')
                                            }}
                                        >
                                            View run
                                        </Button>
                                    ) : null}
                                </div>
                            </InlineNotice>
                        </div>
                    ) : null}
                    {!executionFlowName ? (
                        <div
                            data-testid="execution-no-flow-state"
                            className="flex min-h-0 flex-1 items-center justify-center p-6"
                        >
                            <div className="max-w-md rounded-lg border border-dashed border-border bg-background/70 px-6 py-5 text-center shadow-sm">
                                <p className="text-sm font-medium text-foreground">Select a flow to launch.</p>
                                <p className="mt-2 text-sm text-muted-foreground">
                                    Execution is a launch surface; use Runs to inspect live or completed runs.
                                </p>
                            </div>
                        </div>
                    ) : (
                        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pr-2">
                            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
                                <div className="space-y-1">
                                    <p className="text-sm font-medium text-foreground">
                                        Launch target
                                    </p>
                                    <p
                                        data-testid="execution-launch-target-copy"
                                        className="text-sm text-muted-foreground"
                                    >
                                        {activeProjectPath
                                            ? `${executeLabel} using the active project context.`
                                            : 'Select an active project to enable launching.'}
                                    </p>
                                </div>
                                <div className="flex flex-wrap items-center gap-3">
                                    <div className="flex items-center gap-2">
                                        <Checkbox
                                            id="execution-open-runs-after-launch-checkbox"
                                            checked={openRunsAfterLaunch}
                                            onCheckedChange={(checked) => {
                                                setOpenRunsAfterLaunch(checked === true)
                                            }}
                                        />
                                        <Label
                                            htmlFor="execution-open-runs-after-launch-checkbox"
                                            className="text-xs text-muted-foreground"
                                        >
                                            Open in Runs after launch
                                        </Label>
                                    </div>
                                    <div data-testid="execution-launch-primary-action">
                                        <Button
                                            type="button"
                                            data-testid="execute-button"
                                            onClick={() => {
                                                void requestStart()
                                            }}
                                            disabled={!canRun}
                                            title={canRun ? undefined : executeDisabledReason}
                                        >
                                            {executeLabel}
                                        </Button>
                                    </div>
                                </div>
                            </div>

                            {isLoadingPreview ? (
                                <InlineNotice data-testid="execution-launch-preview-loading">
                                    Loading flow preview and launch contract…
                                </InlineNotice>
                            ) : null}
                            {previewLoadError ? (
                                <InlineNotice data-testid="execution-launch-preview-error" tone="error">
                                    {previewLoadError}
                                </InlineNotice>
                            ) : null}
                            {launchSuccessRunId && !openRunsAfterLaunch ? (
                                <InlineNotice data-testid="execution-launch-success-notice" tone="success">
                                    <div className={`flex ${isNarrowViewport ? 'flex-col items-start gap-2' : 'items-center justify-between gap-3'}`}>
                                        <div>
                                            Run started: <span className="font-mono">{launchSuccessRunId}</span>
                                        </div>
                                        <Button
                                            type="button"
                                            data-testid="execution-launch-success-view-run-button"
                                            variant="outline"
                                            size="xs"
                                            onClick={() => {
                                                setSelectedRunId(launchSuccessRunId)
                                                setViewMode('runs')
                                            }}
                                        >
                                            View run
                                        </Button>
                                    </div>
                                </InlineNotice>
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

                            {visibleDiagnostics.length > 0 ? (
                                <div
                                    data-testid="execution-launch-diagnostics"
                                    className="space-y-2 rounded-md border border-border/70 bg-muted/20 p-3"
                                >
                                    <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                        Launch diagnostics
                                    </p>
                                    <ul className="space-y-2 text-sm">
                                        {visibleDiagnostics.map((diagnostic, index) => (
                                            <li
                                                key={`${diagnostic.rule_id}-${diagnostic.node_id || 'graph'}-${index}`}
                                                className="rounded border border-border/70 bg-background/80 px-3 py-2"
                                            >
                                                <div className="flex flex-wrap items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                                                    <span>{diagnostic.severity}</span>
                                                    <span>{diagnostic.rule_id}</span>
                                                    {diagnostic.node_id ? <span>{diagnostic.node_id}</span> : null}
                                                </div>
                                                <p className="mt-1 text-sm text-foreground">{diagnostic.message}</p>
                                            </li>
                                        ))}
                                    </ul>
                                </div>
                            ) : null}

                            <div className="rounded-lg border border-border/80 bg-muted/10 p-4">
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
                            </div>
                        </div>
                    )}
                </PanelContent>
            </Panel>
        </div>
    )
}
