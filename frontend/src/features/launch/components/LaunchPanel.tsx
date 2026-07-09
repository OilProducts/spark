import { useEffect, useMemo, useState } from 'react'
import { X } from 'lucide-react'

import {
    buildLaunchContextFromValues,
    initializeLaunchInputFormValues,
    launchContextToFormValues,
    parseLaunchInputDefinitions,
    type LaunchInputFormValues,
} from '@/lib/flowContracts'
import { formatProjectListLabel } from '@/features/projects/model/projectsHomeState'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { useExecutionPlacement } from '../hooks/useExecutionPlacement'
import { useFlowLaunchMetadata } from '../hooks/useFlowLaunchMetadata'
import { useLaunchPreview } from '../hooks/useLaunchPreview'
import { launchErrorMessage, useStartPipeline } from '../hooks/useStartPipeline'
import type { LaunchFailureDiagnostics, LaunchTarget } from '../model/launchTypes'
import { ExecutionProfileSection } from './ExecutionProfileSection'
import { LaunchInputsForm } from './LaunchInputsForm'
import { LaunchNoticeStack } from './LaunchNoticeStack'

export interface LaunchPanelProps {
    target: LaunchTarget
    projectPath: string | null
    initialInputValues?: LaunchInputFormValues
    initialLaunchContext?: Record<string, unknown> | null
    initialWorkingDirectory: string
    initialModel?: string
    infoNotice?: string | null
    onLaunched: (runId: string | null, queued: boolean) => void
    onClose: () => void
}

export function LaunchPanel({
    target,
    projectPath,
    initialInputValues,
    initialLaunchContext,
    initialWorkingDirectory,
    initialModel,
    infoNotice,
    onLaunched,
    onClose,
}: LaunchPanelProps) {
    const isNarrowViewport = useNarrowViewport()
    const { startFromFlowContent, logUnexpectedLaunchError } = useStartPipeline()
    const placement = useExecutionPlacement(true)
    const lockMetadata = useFlowLaunchMetadata(target.previewSource.kind === 'flow' ? target.flowName : null)
    const {
        isLoadingPreview,
        previewLoadError,
        hydratedGraph,
        diagnostics,
        hasValidationErrors,
        graphAttrs,
    } = useLaunchPreview(target.previewSource)

    const [launchInputValues, setLaunchInputValues] = useState<LaunchInputFormValues>(initialInputValues ?? {})
    const [launchInputsCollapsed, setLaunchInputsCollapsed] = useState(false)
    const [workingDirectory, setWorkingDirectory] = useState(initialWorkingDirectory)
    const [model, setModel] = useState(initialModel ?? '')
    const [isLaunching, setIsLaunching] = useState(false)
    const [runStartError, setRunStartError] = useState<string | null>(null)
    const [gitPolicyWarning, setGitPolicyWarning] = useState<string | null>(null)
    const [lastLaunchFailure, setLastLaunchFailure] = useState<LaunchFailureDiagnostics | null>(null)

    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(
            hydratedGraph?.flowInputs.length ? hydratedGraph.flowInputs : graphAttrs.inputs,
        ),
        [graphAttrs, hydratedGraph?.flowInputs],
    )

    useEffect(() => {
        setLaunchInputValues((previous) => {
            const seeded = initialInputValues
                ?? (initialLaunchContext
                    ? launchContextToFormValues(parsedLaunchInputs.entries, initialLaunchContext)
                    : undefined)
            const merged = seeded ? { ...seeded, ...previous } : previous
            const nextValues = initializeLaunchInputFormValues(parsedLaunchInputs.entries, merged)
            const sameKeys = Object.keys(nextValues).length === Object.keys(previous).length
                && Object.entries(nextValues).every(([key, value]) => previous[key] === value)
            return sameKeys ? previous : nextValues
        })
    }, [initialInputValues, initialLaunchContext, parsedLaunchInputs.entries])

    const showValidationWarningBanner = diagnostics.some((diag) => diag.severity === 'warning') && !hasValidationErrors
    const visibleDiagnostics = diagnostics.slice(0, 8)
    const executeLabel = projectPath ? `Run in ${formatProjectListLabel(projectPath)}` : 'Run'
    const executeDisabledReason = !projectPath
        ? 'Select an active project before running.'
        : isLoadingPreview
            ? 'Loading flow preview for launch inputs.'
            : hasValidationErrors
                ? 'Fix validation errors before running.'
                : placement.validationMessage
                    ? placement.validationMessage
                    : parsedLaunchInputs.error
                        ? 'Fix launch-input schema errors before running.'
                        : isLaunching
                            ? 'Launch in progress.'
                            : undefined
    const canRun = !executeDisabledReason
    const canRetryLaunch = Boolean(projectPath) && !hasValidationErrors && !isLaunching

    const selectedLlmProvider = typeof graphAttrs.llm_provider === 'string' ? graphAttrs.llm_provider : ''
    const selectedLlmProfile = typeof graphAttrs.llm_profile === 'string' ? graphAttrs.llm_profile : ''
    const selectedReasoningEffort = typeof graphAttrs.reasoning_effort === 'string' ? graphAttrs.reasoning_effort : ''

    const requestStart = async () => {
        if (!canRun || !projectPath) {
            return
        }
        setRunStartError(null)
        setIsLaunching(true)
        try {
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

            const flowContent = await target.loadFlowContent()
            const result = await startFromFlowContent(
                {
                    projectPath,
                    flowSource: target.flowName || '',
                    workingDirectory,
                    model: model.trim() || null,
                    llmProvider: selectedLlmProfile ? '' : selectedLlmProvider,
                    llmProfile: selectedLlmProfile,
                    reasoningEffort: selectedReasoningEffort,
                    launchContext,
                    executionProfileId: placement.selectedProfileId || null,
                    projectDefaultExecutionProfileId: placement.projectDefaultProfileId,
                },
                flowContent,
                { onGitPolicyWarning: setGitPolicyWarning },
            )
            if (result.status === 'cancelled') {
                return
            }
            setLastLaunchFailure(null)
            onLaunched(result.runId, result.queued)
        } catch (error) {
            logUnexpectedLaunchError(error)
            const errorMessage = launchErrorMessage(error)
            setRunStartError(errorMessage)
            setLastLaunchFailure({
                message: errorMessage,
                failedAt: new Date().toISOString(),
                flowSource: target.flowName || null,
            })
        } finally {
            setIsLaunching(false)
        }
    }

    return (
        <div data-testid="launch-panel" className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
            <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 space-y-1">
                    <h3 className="text-sm font-semibold text-foreground">Launch Flow</h3>
                    <p
                        data-testid="launch-panel-target-copy"
                        className="text-xs leading-5 text-muted-foreground"
                    >
                        {projectPath
                            ? `${executeLabel} using the active project context.`
                            : 'Select an active project to enable launching.'}
                    </p>
                    {target.flowName ? (
                        <span
                            data-testid="launch-panel-flow-name"
                            className="block max-w-[20rem] truncate font-mono text-xs text-muted-foreground"
                            title={target.flowName}
                        >
                            {target.flowName}
                        </span>
                    ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                    <Button
                        type="button"
                        data-testid="launch-panel-start-button"
                        onClick={() => {
                            void requestStart()
                        }}
                        disabled={!canRun}
                        title={canRun ? undefined : executeDisabledReason}
                    >
                        {executeLabel}
                    </Button>
                    <Button
                        type="button"
                        data-testid="launch-panel-close-button"
                        variant="ghost"
                        size="icon-xs"
                        aria-label="Close launch panel"
                        onClick={onClose}
                    >
                        <X className="h-3.5 w-3.5" />
                    </Button>
                </div>
            </div>

            {infoNotice ? (
                <Alert
                    data-testid="launch-panel-info-notice"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">{infoNotice}</AlertDescription>
                </Alert>
            ) : null}
            {isLoadingPreview ? (
                <Alert
                    data-testid="launch-panel-preview-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        Loading flow preview and launch contract…
                    </AlertDescription>
                </Alert>
            ) : null}
            {previewLoadError ? (
                <Alert
                    data-testid="launch-panel-preview-error"
                    className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                >
                    <AlertDescription className="text-inherit">{previewLoadError}</AlertDescription>
                </Alert>
            ) : null}
            {lockMetadata?.execution_lock ? (
                <Alert
                    data-testid="execution-launch-lock-notice"
                    className="border-amber-500/40 bg-amber-500/10 px-3 py-2 text-amber-800"
                >
                    <AlertDescription className="text-inherit">
                        Execution lock: {lockMetadata.execution_lock.scope} / {lockMetadata.execution_lock.key} / {lockMetadata.execution_lock.conflict_policy}. This launch policy is stored in the workspace flow catalog, not in YAML.
                    </AlertDescription>
                </Alert>
            ) : null}

            <LaunchNoticeStack
                showValidationWarningBanner={showValidationWarningBanner}
                runStartGitPolicyWarning={gitPolicyWarning}
                runStartError={runStartError}
                lastLaunchFailure={lastLaunchFailure}
                canRetryLaunch={canRetryLaunch}
                onRetry={() => {
                    void requestStart()
                }}
            />

            <ExecutionProfileSection placement={placement} />

            <div className="grid gap-3 md:grid-cols-2">
                <div className="space-y-1.5">
                    <Label htmlFor="launch-panel-working-directory" className="text-xs">
                        Working directory
                    </Label>
                    <Input
                        id="launch-panel-working-directory"
                        data-testid="launch-panel-working-directory-input"
                        value={workingDirectory}
                        onChange={(event) => setWorkingDirectory(event.target.value)}
                        placeholder={projectPath || undefined}
                        className="h-8 text-xs"
                    />
                </div>
                <div className="space-y-1.5">
                    <Label htmlFor="launch-panel-model" className="text-xs">
                        Model override
                    </Label>
                    <Input
                        id="launch-panel-model"
                        data-testid="launch-panel-model-input"
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                        placeholder="Use flow/server default"
                        className="h-8 text-xs"
                    />
                </div>
            </div>

            {visibleDiagnostics.length > 0 ? (
                <div
                    data-testid="launch-panel-diagnostics"
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
                <LaunchInputsForm
                    isNarrowViewport={isNarrowViewport}
                    flowName={target.flowName}
                    parsedLaunchInputs={parsedLaunchInputs}
                    launchInputValues={launchInputValues}
                    launchInputsCollapsed={launchInputsCollapsed}
                    onToggleCollapsed={() => setLaunchInputsCollapsed((collapsed) => !collapsed)}
                    onInputChange={(entry, value) => {
                        setLaunchInputValues((previous) => ({
                            ...previous,
                            [entry.key]: value,
                        }))
                    }}
                />
            </div>
        </div>
    )
}
