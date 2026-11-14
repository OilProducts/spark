import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronRight, X } from 'lucide-react'

import {
    buildLaunchContextFromValues,
    initializeLaunchInputFormValues,
    launchContextToFormValues,
    parseLaunchInputDefinitions,
    type LaunchInputFormValues,
} from '@/lib/flowContracts'
import { projectLabel } from '@/features/projects/model/projectChoices'
import { flowTitle } from '@/features/runs/model/runOverviewModel'
import { useStore } from '@/store'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { InlineError } from '@/components/app/inline-error'
import { Well } from '@/components/app/well'
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
    /** Omitted where the panel's container closes it, as a dialog does with its own button and Escape. */
    onClose?: () => void
    /** Opens the flow in the editor; offered beside its errors, where the panel isn't already in the editor. */
    onEditFlow?: () => void
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
    onEditFlow,
}: LaunchPanelProps) {
    const { startFlow, logUnexpectedLaunchError } = useStartPipeline()
    const placement = useExecutionPlacement(true, projectPath)
    const lockMetadata = useFlowLaunchMetadata(target.previewSource.kind === 'flow' ? target.flowName : null, projectPath)
    const {
        isLoadingPreview,
        previewLoadError,
        hydratedGraph,
        diagnostics,
        hasValidationErrors,
        graphAttrs,
    } = useLaunchPreview(target.previewSource, projectPath)

    const [launchInputValues, setLaunchInputValues] = useState<LaunchInputFormValues>(initialInputValues ?? {})
    const [inputErrors, setInputErrors] = useState<Record<string, string>>({})
    const [launchInputsCollapsed, setLaunchInputsCollapsed] = useState(false)
    const [workingDirectory, setWorkingDirectory] = useState(initialWorkingDirectory)
    const [model, setModel] = useState(initialModel ?? '')
    const [runName, setRunName] = useState('')
    const [isLaunching, setIsLaunching] = useState(false)
    const [runStartError, setRunStartError] = useState<string | null>(null)
    const [gitPolicyWarning, setGitPolicyWarning] = useState<string | null>(null)
    const [lastLaunchFailure, setLastLaunchFailure] = useState<LaunchFailureDiagnostics | null>(null)
    const [optionsOpenState, setOptionsOpen] = useState(false)
    const panelRef = useRef<HTMLDivElement>(null)

    // Take focus on open so Escape works immediately; hand it back to the opener on close.
    useEffect(() => {
        const opener = document.activeElement
        panelRef.current?.focus()
        return () => {
            if (opener instanceof HTMLElement) opener.focus()
        }
    }, [])

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
    const projectRegistry = useStore((state) => state.projectRegistry)
    const executeLabel = projectPath ? `Run in ${projectLabel(projectRegistry, projectPath)}` : 'Run'
    const executeDisabledReason = !projectPath
        ? 'Choose a project before running.'
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
    // A placement problem blocks the run, so its controls show without asking.
    const optionsOpen = optionsOpenState || Boolean(placement.validationMessage)
    const panelTitle = lockMetadata?.title
        || (target.previewSource.kind === 'runSnapshot' ? target.previewSource.displayName : null)
        || (target.flowName ? flowTitle(target.flowName) : 'Run a flow')
    const flowDescription = lockMetadata?.description?.trim() || null
    const optionsSummary = [
        projectPath ? projectLabel(projectRegistry, projectPath) : 'no project',
        placement.effectiveProfileSummary,
        workingDirectory.trim() && workingDirectory.trim() !== projectPath ? `in ${workingDirectory.trim()}` : null,
        model.trim() || null,
    ].filter(Boolean).join(' · ')
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
            setInputErrors(launchContextErrors)
            const firstInvalid = parsedLaunchInputs.entries.find((entry) => launchContextErrors[entry.key])
            if (firstInvalid) {
                // Each problem shows under its field; focus goes to the first so it can be fixed at once.
                setLaunchInputsCollapsed(false)
                window.requestAnimationFrame(() => {
                    panelRef.current?.querySelector<HTMLElement>(`[data-testid="execution-launch-input-${CSS.escape(firstInvalid.key)}"]`)?.focus()
                })
                return
            }

            if (!target.flowName) {
                setRunStartError('This run has no installed flow to launch.')
                return
            }
            await target.prepareLaunch?.()
            const result = await startFlow(
                {
                    projectPath,
                    flowSource: target.flowName,
                    workingDirectory,
                    model: model.trim() || null,
                    llmProvider: selectedLlmProfile ? '' : selectedLlmProvider,
                    llmProfile: selectedLlmProfile,
                    reasoningEffort: selectedReasoningEffort,
                    launchContext,
                    executionProfileId: placement.selectedProfileId || null,
                    title: runName,
                },
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
        <div
            ref={panelRef}
            tabIndex={-1}
            data-testid="launch-panel"
            className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto outline-none"
            onKeyDown={(event) => {
                if (onClose && event.key === 'Escape' && !event.defaultPrevented && !event.nativeEvent.isComposing) {
                    event.stopPropagation()
                    onClose()
                }
            }}
        >
            <div className="space-y-1">
                {/* The title wraps rather than give way, and the close button keeps its corner at any width. */}
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                        <h3 data-testid="launch-panel-title" className="break-words text-lg font-semibold leading-tight text-foreground">{panelTitle}</h3>
                        {target.flowName ? (
                            <span
                                data-testid="launch-panel-flow-name"
                                className="mt-0.5 block truncate font-mono text-xs text-muted-foreground"
                                title={target.flowName}
                            >
                                {target.flowName}
                            </span>
                        ) : null}
                    </div>
                    {onClose ? (
                        <Button
                            type="button"
                            data-testid="launch-panel-close-button"
                            variant="ghost"
                            size="icon-xs"
                            aria-label="Close launch panel"
                            onClick={onClose}
                            className="shrink-0"
                        >
                            <X className="h-3.5 w-3.5" />
                        </Button>
                    ) : null}
                </div>
                {flowDescription ? (
                    <p data-testid="launch-panel-description" className="text-sm text-muted-foreground">{flowDescription}</p>
                ) : null}
            </div>

            {infoNotice ? (
                <Alert
                    data-testid="launch-panel-info-notice"
                    className="border-border/70 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">{infoNotice}</AlertDescription>
                </Alert>
            ) : null}
            {isLoadingPreview ? (
                <p data-testid="launch-panel-preview-loading" className="text-sm text-muted-foreground" aria-live="polite">Loading the flow…</p>
            ) : null}
            {previewLoadError ? (
                <InlineError data-testid="launch-panel-preview-error">{previewLoadError}</InlineError>
            ) : null}
            {lockMetadata?.execution_lock ? (
                <Alert
                    data-testid="execution-launch-lock-notice"
                    className="border-warning px-3 py-2 text-warning"
                >
                    <AlertDescription className="text-inherit">
                        One run of this flow at a time per project: if another is running in this project, this one waits its turn.
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

            {parsedLaunchInputs.entries.length > 0 || parsedLaunchInputs.error ? <Well>
                <LaunchInputsForm
                    flowName={target.flowName}
                    parsedLaunchInputs={parsedLaunchInputs}
                    launchInputValues={launchInputValues}
                    errors={inputErrors}
                    launchInputsCollapsed={launchInputsCollapsed}
                    onToggleCollapsed={() => setLaunchInputsCollapsed((collapsed) => !collapsed)}
                    onInputChange={(entry, value) => {
                        setLaunchInputValues((previous) => ({
                            ...previous,
                            [entry.key]: value,
                        }))
                        setInputErrors((previous) => {
                            if (!previous[entry.key]) return previous
                            const { [entry.key]: _fixed, ...rest } = previous
                            return rest
                        })
                    }}
                />
            </Well> : !isLoadingPreview && !previewLoadError && !hasValidationErrors ? (
                <p data-testid="launch-panel-no-inputs" className="text-sm text-muted-foreground">This flow needs no inputs to start.</p>
            ) : null}

            {/* Runs of one flow otherwise share its name; a name here tells this one apart everywhere it is listed.
                Optional, so it follows what the flow needs. */}
            <div className="space-y-1.5">
                <Label htmlFor="launch-panel-run-name" className="text-sm">
                    Run name <span className="font-normal text-muted-foreground">(optional)</span>
                </Label>
                <Input
                    id="launch-panel-run-name"
                    data-testid="launch-panel-run-name-input"
                    value={runName}
                    maxLength={120}
                    onChange={(event) => setRunName(event.target.value)}
                    aria-describedby="launch-panel-run-name-hint"
                    className="h-8 text-sm"
                />
                <p id="launch-panel-run-name-hint" className="text-xs text-muted-foreground">Tells this run apart in Runs, Needs you and the Inbox. Left empty, it is named from {parsedLaunchInputs.entries.length > 0 ? <>its inputs, or listed as {panelTitle} without them</> : <>the flow, as {panelTitle}</>}.</p>
            </div>

            {/* Where and how it runs: rarely changed, so folded under one line that says what it will use. */}
            <details
                data-testid="launch-panel-options"
                open={optionsOpen}
                onToggle={(event) => {
                    const details = event.currentTarget
                    setOptionsOpen(details.open)
                    // Opened at the foot of a short dialog, the options scroll into view above the Run button.
                    if (details.open) details.scrollIntoView?.({ block: 'nearest' })
                }}
                className="group scroll-mb-20 rounded-lg border border-border"
            >
                {/* On a phone the summary takes a line of its own rather than being cut short. */}
                <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-2 rounded-lg px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
                    <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
                    <span className="shrink-0 whitespace-nowrap font-medium">Run options</span>
                    <span data-testid="launch-panel-options-summary" className="min-w-0 truncate text-muted-foreground max-sm:basis-full max-sm:whitespace-normal max-sm:pl-5.5">{optionsSummary}</span>
                </summary>
                <div className="space-y-3 border-t border-border px-3 py-3">
                    {/* The project folder shows once, as the working directory's placeholder. */}
                    {projectPath ? null : (
                        <p data-testid="launch-panel-target-copy" className="text-xs leading-5 text-muted-foreground">Choose a project to enable launching.</p>
                    )}
                    <ExecutionProfileSection placement={placement} />
                    <div className="space-y-1.5">
                        <Label htmlFor="launch-panel-working-directory" className="text-sm">
                            Working directory
                        </Label>
                        <Input
                            id="launch-panel-working-directory"
                            data-testid="launch-panel-working-directory-input"
                            value={workingDirectory}
                            onChange={(event) => setWorkingDirectory(event.target.value)}
                            placeholder={projectPath || undefined}
                            aria-describedby="launch-panel-working-directory-hint"
                            className="h-8 text-sm"
                        />
                        <p id="launch-panel-working-directory-hint" className="text-xs text-muted-foreground">The folder the run works in; leave it as the project folder unless the flow should work elsewhere.</p>
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="launch-panel-model" className="text-sm">
                            Model
                        </Label>
                        <Input
                            id="launch-panel-model"
                            data-testid="launch-panel-model-input"
                            value={model}
                            onChange={(event) => setModel(event.target.value)}
                            placeholder="The flow's own choice"
                            className="h-8 text-sm"
                        />
                    </div>
                </div>
            </details>

            {visibleDiagnostics.length > 0 ? (
                <Well
                    data-testid="launch-panel-diagnostics"
                    className="space-y-2 p-3"
                >
                    <div className="flex flex-wrap items-center justify-between gap-2">
                        <p className="text-sm font-semibold text-foreground">
                            Problems found in this flow
                        </p>
                        {onEditFlow && hasValidationErrors ? (
                            <Button type="button" size="sm" variant="outline" data-testid="launch-panel-edit-flow" onClick={onEditFlow}>
                                Fix in the editor
                            </Button>
                        ) : null}
                    </div>
                    <ul className="space-y-2 text-sm">
                        {visibleDiagnostics.map((diagnostic, index) => (
                            <li
                                key={`${diagnostic.rule_id}-${diagnostic.node_id || 'graph'}-${index}`}
                                className="rounded-lg bg-background px-3 py-2"
                            >
                                <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                                    <span className="capitalize">{diagnostic.severity}</span>
                                    {diagnostic.node_id ? <span>· node {diagnostic.node_id}</span> : null}
                                </div>
                                <p className="mt-1 text-sm text-foreground">{diagnostic.message}</p>
                            </li>
                        ))}
                    </ul>
                </Well>
            ) : null}

            {/* Fill in the form, then run: the button sits under it and stays in view while the form scrolls. */}
            <div className="sticky bottom-0 -mb-px mt-auto flex flex-wrap items-center justify-end gap-x-3 gap-y-2 border-t border-border bg-background pt-3">
                {!canRun && executeDisabledReason && !isLoadingPreview && !isLaunching ? (
                    <p data-testid="launch-panel-disabled-reason" className="min-w-0 flex-1 text-xs text-muted-foreground">{executeDisabledReason}</p>
                ) : null}
                <Button
                    type="button"
                    data-testid="launch-panel-start-button"
                    onClick={() => {
                        void requestStart()
                    }}
                    disabled={!canRun}
                    title={canRun ? executeLabel : executeDisabledReason}
                    className="min-w-0 max-w-full"
                >
                    <span className="truncate">{isLaunching ? 'Starting…' : executeLabel}</span>
                </Button>
            </div>
        </div>
    )
}
