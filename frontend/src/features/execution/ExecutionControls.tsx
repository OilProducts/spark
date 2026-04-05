import { useEffect, useMemo } from 'react'

import {
    buildLaunchContextFromValues,
    initializeLaunchInputFormValues,
    parseLaunchInputDefinitions,
} from '@/lib/flowContracts'
import {
    buildPipelineContinuePayload,
    buildPipelineStartPayload,
    type PipelineStartPayload,
} from '@/lib/pipelineStartPayload'
import { formatProjectListLabel } from '@/features/projects/model/projectsHomeState'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
import {
    Button,
    Checkbox,
    InlineNotice,
    Input,
    Label,
    Panel,
    PanelContent,
    PanelHeader,
    SectionHeader,
    useDialogController,
} from '@/ui'

import { ExecutionGraphCard } from './components/ExecutionGraphCard'
import { ExecutionLaunchInputsSurface } from './components/ExecutionLaunchInputsSurface'
import { ExecutionNoticeStack } from './components/ExecutionNoticeStack'
import { useExecutionLaunchPreview } from './hooks/useExecutionLaunchPreview'
import {
    ApiHttpError,
    continueExecutionRun,
    loadExecutionFlowPayload,
    loadExecutionProjectMetadata,
    startExecutionRun,
} from './services/executionRunService'

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
    const executionContinuation = useStore((state) => state.executionContinuation)
    const clearExecutionContinuation = useStore((state) => state.clearExecutionContinuation)
    const setExecutionContinuationFlowSourceMode = useStore((state) => state.setExecutionContinuationFlowSourceMode)
    const setExecutionContinuationStartNode = useStore((state) => state.setExecutionContinuationStartNode)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const graphAttrs = useStore((state) => state.executionGraphAttrs)
    const diagnostics = useStore((state) => state.executionDiagnostics)
    const hasValidationErrors = useStore((state) => state.executionHasValidationErrors)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const humanGate = useStore((state) => state.humanGate)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const runsScopeMode = useStore((state) => state.runsListSession.scopeMode)
    const setRunsSelectedRunIdForScope = useStore((state) => state.setRunsSelectedRunIdForScope)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const setRuntimeOutcome = useStore((state) => state.setRuntimeOutcome)
    const setViewMode = useStore((state) => state.setViewMode)
    const launchInputValues = useStore((state) => state.executionLaunchInputValues)
    const runStartError = useStore((state) => state.executionLaunchError)
    const lastLaunchFailure = useStore((state) => state.executionLastLaunchFailure)
    const runStartGitPolicyWarning = useStore((state) => state.executionRunStartGitPolicyWarning)
    const collapsedLaunchInputsByFlow = useStore((state) => state.executionCollapsedLaunchInputsByFlow)
    const openRunsAfterLaunch = useStore((state) => state.executionOpenRunsAfterLaunch)
    const launchSuccessRunId = useStore((state) => state.executionLaunchSuccessRunId)
    const updateExecutionSession = useStore((state) => state.updateExecutionSession)
    const isNarrowViewport = useNarrowViewport()

    const executionFlowName = executionFlow
    const isContinuationMode = Boolean(executionContinuation)
    const { isLoadingPreview, previewLoadError, hydratedGraph } = useExecutionLaunchPreview(
        executionFlowName,
        executionContinuation,
    )
    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(graphAttrs['spark.launch_inputs']),
        [graphAttrs],
    )
    const launchInputCount = parsedLaunchInputs.entries.length
    const launchInputsCollapsed = executionFlowName ? (collapsedLaunchInputsByFlow[executionFlowName] ?? false) : false
    const canCollapseLaunchInputs = parsedLaunchInputs.entries.length > 0
    const showValidationWarningBanner = diagnostics.some((diag) => diag.severity === 'warning') && !hasValidationErrors
    const executeLabel = isContinuationMode
        ? activeProjectPath
            ? `Continue in ${formatProjectListLabel(activeProjectPath)}`
            : 'Continue from node'
        : activeProjectPath
            ? `Run in ${formatProjectListLabel(activeProjectPath)}`
            : 'Run'
    const executeDisabledReason = isContinuationMode
        ? isLoadingPreview
            ? 'Loading graph preview for continuation.'
            : hasValidationErrors
                ? 'Fix validation errors before continuing.'
                : executionContinuation?.flowSourceMode === 'flow_name' && !executionFlowName
                    ? 'Select an installed flow override or switch back to the source snapshot.'
                    : !executionContinuation?.startNodeId
                        ? 'Select a restart node in the graph.'
                        : undefined
        : !activeProjectPath
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
    const canRetryLaunch = isContinuationMode
        ? Boolean(executionContinuation?.startNodeId) && !hasValidationErrors
        : Boolean(activeProjectPath) && Boolean(executionFlowName) && !hasValidationErrors
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
        const nextValues = initializeLaunchInputFormValues(parsedLaunchInputs.entries, launchInputValues)
        const sameKeys = Object.keys(nextValues).length === Object.keys(launchInputValues).length
            && Object.entries(nextValues).every(([key, value]) => launchInputValues[key] === value)
        if (sameKeys) {
            return
        }
        updateExecutionSession({ executionLaunchInputValues: nextValues })
    }, [launchInputValues, parsedLaunchInputs.entries, updateExecutionSession])

    useEffect(() => {
        updateExecutionSession({
            executionLaunchError: null,
            executionLastLaunchFailure: null,
            executionRunStartGitPolicyWarning: null,
            executionLaunchSuccessRunId: null,
        })
    }, [executionFlowName, executionContinuation, updateExecutionSession])

    const confirmGitPolicyGate = async () => {
        const projectPathForGitCheck = activeProjectPath || executionContinuation?.sourceWorkingDirectory || ''
        if (!projectPathForGitCheck) {
            return true
        }

        try {
            const metadata = await loadExecutionProjectMetadata(projectPathForGitCheck)
            const branch = typeof metadata.branch === 'string' ? metadata.branch.trim() : ''
            if (branch) {
                updateExecutionSession({ executionRunStartGitPolicyWarning: null })
                return true
            }

            const warning = 'Project Git policy check failed: active project is not a Git repository.'
            updateExecutionSession({ executionRunStartGitPolicyWarning: warning })
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
            updateExecutionSession({ executionRunStartGitPolicyWarning: warning })
            return confirm({
                title: 'Unable to verify Git state',
                description: `${warning} Continue with run start anyway?`,
                confirmLabel: 'Continue',
                cancelLabel: 'Cancel',
            })
        }
    }

    const requestStart = async () => {
        if (!canRun) {
            return
        }

        updateExecutionSession({
            executionLaunchError: null,
            executionLaunchSuccessRunId: null,
        })

        try {
            const gitPolicyGateAllowed = await confirmGitPolicyGate()
            if (!gitPolicyGateAllowed) {
                return
            }

            let runData
            if (isContinuationMode && executionContinuation) {
                const continuePayload = buildPipelineContinuePayload(
                    {
                        projectPath: activeProjectPath || executionContinuation.sourceWorkingDirectory,
                        workingDirectory: workingDir,
                        model: model.trim() || null,
                    },
                    {
                        startNodeId: executionContinuation.startNodeId || '',
                        flowSourceMode: executionContinuation.flowSourceMode,
                        flowName: executionContinuation.flowSourceMode === 'flow_name' ? executionFlowName : null,
                    },
                )
                runData = await continueExecutionRun(executionContinuation.sourceRunId, continuePayload)
            } else {
                if (!activeProjectPath || !executionFlowName) {
                    return
                }
                if (parsedLaunchInputs.error) {
                    updateExecutionSession({
                        executionLaunchError: `Flow launch input schema is invalid: ${parsedLaunchInputs.error}`,
                    })
                    return
                }

                const { launchContext, errors: launchContextErrors } = buildLaunchContextFromValues(
                    parsedLaunchInputs.entries,
                    launchInputValues,
                )
                if (launchContextErrors.length > 0) {
                    updateExecutionSession({
                        executionLaunchError: launchContextErrors.join(' '),
                    })
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
                runData = await startExecutionRun(startPayload as PipelineStartPayload)
            }

            if (runData?.status !== 'started') {
                const reason = runData?.error || runData?.status || 'Unknown run error'
                throw new Error(`Run not started: ${reason}`)
            }

            const nextRunId = typeof runData?.pipeline_id === 'string' ? runData.pipeline_id : null
            if (nextRunId) {
                setRunsSelectedRunIdForScope(
                    buildRunsScopeKey(runsScopeMode, activeProjectPath),
                    nextRunId,
                )
                setSelectedRunId(nextRunId)
                updateExecutionSession({ executionLaunchSuccessRunId: nextRunId })
            }
            setRuntimeStatus('running')
            setRuntimeOutcome(null)
            updateExecutionSession({ executionLastLaunchFailure: null })

            if (isContinuationMode && nextRunId) {
                clearExecutionContinuation()
                updateExecutionSession({ executionLaunchSuccessRunId: null })
                setViewMode('runs')
            } else if (openRunsAfterLaunch && nextRunId) {
                setViewMode('runs')
            }
        } catch (error) {
            logUnexpectedExecutionError(error)
            const errorMessage = error instanceof ApiHttpError && error.detail
                ? error.detail
                : error instanceof Error
                    ? error.message
                    : 'Failed to start pipeline run.'
            updateExecutionSession({
                executionLaunchError: errorMessage,
                executionLastLaunchFailure: {
                    message: errorMessage,
                    failedAt: new Date().toISOString(),
                    flowSource: isContinuationMode
                        ? executionContinuation?.sourceRunId || null
                        : runInitiationForm.flowSource || null,
                },
            })
        }
    }

    return (
        <div data-testid="execution-launch-panel" className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
            <Panel className="m-4 flex min-h-0 flex-1 flex-col overflow-hidden">
                <PanelHeader>
                    <SectionHeader
                        title={isContinuationMode ? 'Continue Run' : 'Launch Flow'}
                        description={isContinuationMode
                            ? 'Configure a derived run and pick the restart node below.'
                            : executionFlowName
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
                    {!executionFlowName && !isContinuationMode ? (
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
                                        {isContinuationMode ? 'Continuation target' : 'Launch target'}
                                    </p>
                                    <p
                                        data-testid="execution-launch-target-copy"
                                        className="text-sm text-muted-foreground"
                                    >
                                        {isContinuationMode && executionContinuation
                                            ? `Create a derived run from ${executionContinuation.sourceRunId} using inherited checkpoint context.`
                                            : activeProjectPath
                                                ? `${executeLabel} using the active project context.`
                                                : 'Select an active project to enable launching.'}
                                    </p>
                                </div>
                                <div className="flex flex-wrap items-center gap-3">
                                    {!isContinuationMode ? (
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id="execution-open-runs-after-launch-checkbox"
                                                checked={openRunsAfterLaunch}
                                                onCheckedChange={(checked) => {
                                                    updateExecutionSession({ executionOpenRunsAfterLaunch: checked === true })
                                                }}
                                            />
                                            <Label
                                                htmlFor="execution-open-runs-after-launch-checkbox"
                                                className="text-xs text-muted-foreground"
                                            >
                                                Open in Runs after launch
                                            </Label>
                                        </div>
                                    ) : null}
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
                                    {isContinuationMode ? 'Loading continuation graph preview…' : 'Loading flow preview and launch contract…'}
                                </InlineNotice>
                            ) : null}
                            {previewLoadError ? (
                                <InlineNotice data-testid="execution-launch-preview-error" tone="error">
                                    {previewLoadError}
                                </InlineNotice>
                            ) : null}
                            {launchSuccessRunId && !openRunsAfterLaunch && !isContinuationMode ? (
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

                            {isContinuationMode && executionContinuation ? (
                                <div
                                    data-testid="execution-continuation-settings"
                                    className="space-y-4 rounded-lg border border-border/80 bg-muted/10 p-4"
                                >
                                    <div className="grid gap-4 md:grid-cols-2">
                                        <div className="space-y-1">
                                            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                                Source run
                                            </p>
                                            <p data-testid="execution-continuation-source-run" className="font-mono text-sm text-foreground">
                                                {executionContinuation.sourceRunId}
                                            </p>
                                        </div>
                                        <div className="space-y-2">
                                            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                                Graph source
                                            </p>
                                            <div className="flex flex-wrap gap-2">
                                                <Button
                                                    type="button"
                                                    size="xs"
                                                    variant={executionContinuation.flowSourceMode === 'snapshot' ? 'secondary' : 'outline'}
                                                    data-testid="execution-continuation-use-snapshot-button"
                                                    onClick={() => {
                                                        setExecutionContinuationFlowSourceMode('snapshot')
                                                        setExecutionContinuationStartNode(null)
                                                    }}
                                                >
                                                    Use source snapshot
                                                </Button>
                                                <span className="text-xs text-muted-foreground">
                                                    Pick a flow in the sidebar to use an installed-flow override.
                                                </span>
                                            </div>
                                            <p data-testid="execution-continuation-flow-source-copy" className="text-xs text-muted-foreground">
                                                {executionContinuation.flowSourceMode === 'snapshot'
                                                    ? 'Currently previewing the stored source-run graph snapshot.'
                                                    : executionFlowName
                                                        ? `Currently previewing installed flow override ${executionFlowName}.`
                                                        : 'Select an installed flow in the sidebar to preview an override.'}
                                            </p>
                                        </div>
                                        <div className="space-y-2">
                                            <Label htmlFor="execution-continuation-working-directory">Working directory</Label>
                                            <Input
                                                id="execution-continuation-working-directory"
                                                data-testid="execution-continuation-working-directory-input"
                                                value={workingDir}
                                                onChange={(event) => setWorkingDir(event.target.value)}
                                            />
                                        </div>
                                        <div className="space-y-2">
                                            <Label htmlFor="execution-continuation-model">Model override</Label>
                                            <Input
                                                id="execution-continuation-model"
                                                data-testid="execution-continuation-model-input"
                                                value={model}
                                                onChange={(event) => setModel(event.target.value)}
                                                placeholder="Use server default"
                                            />
                                        </div>
                                    </div>
                                    <div
                                        data-testid="execution-continuation-selected-node-copy"
                                        className="rounded-md border border-border/80 bg-background/80 px-3 py-2 text-sm text-muted-foreground"
                                    >
                                        {executionContinuation.startNodeId
                                            ? <>Restart node: <span className="font-mono text-foreground">{executionContinuation.startNodeId}</span></>
                                            : 'Select a restart node in the graph below.'}
                                    </div>
                                </div>
                            ) : null}

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

                            {!isContinuationMode ? (
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
                                            updateExecutionSession({
                                                executionCollapsedLaunchInputsByFlow: {
                                                    ...collapsedLaunchInputsByFlow,
                                                    [executionFlowName]: !launchInputsCollapsed,
                                                },
                                            })
                                        }}
                                        onInputChange={(entry, value) => {
                                            updateExecutionSession({
                                                executionLaunchInputValues: {
                                                    ...launchInputValues,
                                                    [entry.key]: value,
                                                },
                                            })
                                        }}
                                    />
                                </div>
                            ) : null}

                            {(executionFlowName || isContinuationMode) ? (
                                <ExecutionGraphCard
                                    hydratedGraph={hydratedGraph}
                                    isLoading={isLoadingPreview}
                                    loadError={previewLoadError}
                                    isContinuationMode={isContinuationMode}
                                    sourceMode={executionContinuation?.flowSourceMode ?? null}
                                    selectedStartNodeId={executionContinuation?.startNodeId ?? null}
                                    onSelectStartNode={(nodeId) => {
                                        setExecutionContinuationStartNode(nodeId)
                                    }}
                                />
                            ) : null}
                        </div>
                    )}
                </PanelContent>
            </Panel>
        </div>
    )
}
