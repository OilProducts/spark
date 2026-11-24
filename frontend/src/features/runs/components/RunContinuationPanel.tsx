import { useEffect, useState } from 'react'

import { InlineError } from '@/components/app/inline-error'
import { SegmentedControl } from '@/components/app/segmented-control'
import { Well } from '@/components/app/well'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
    launchErrorMessage,
    LaunchNoticeStack,
    loadFlowCatalog,
    useLaunchPreview,
    useStartPipeline,
    type ContinuationDraft,
    type LaunchFailureDiagnostics,
} from '@/features/launch'

interface RunContinuationPanelProps {
    draft: ContinuationDraft
    /** The project the source run belongs to. */
    projectPath: string | null
    /** The chosen start step's label. */
    startNodeLabel: string | null
    onDraftChange: (patch: Partial<ContinuationDraft>) => void
    onCancel: () => void
    onContinued: (runId: string | null) => void
}

export function RunContinuationPanel({
    draft,
    projectPath,
    startNodeLabel,
    onDraftChange,
    onCancel,
    onContinued,
}: RunContinuationPanelProps) {
    const { continueFromRun, logUnexpectedLaunchError } = useStartPipeline()
    const [flowCatalog, setFlowCatalog] = useState<string[]>([])
    const [isContinuing, setIsContinuing] = useState(false)
    const [continueError, setContinueError] = useState<string | null>(null)
    const [gitPolicyWarning, setGitPolicyWarning] = useState<string | null>(null)
    const [lastLaunchFailure, setLastLaunchFailure] = useState<LaunchFailureDiagnostics | null>(null)

    const isInstalledFlowMode = draft.flowSourceMode === 'flow_name'
    const previewSource = isInstalledFlowMode
        ? draft.overrideFlowName
            ? { kind: 'flow' as const, flowName: draft.overrideFlowName }
            : null
        : { kind: 'runSnapshot' as const, runId: draft.sourceRunId, displayName: draft.sourceFlowName }
    const {
        isLoadingPreview,
        previewLoadError,
        hydratedGraph,
        diagnostics,
        hasValidationErrors,
        graphAttrs,
    } = useLaunchPreview(previewSource, projectPath)

    useEffect(() => {
        let cancelled = false
        loadFlowCatalog(projectPath)
            .then((flows) => {
                if (!cancelled) {
                    setFlowCatalog(flows)
                }
            })
            .catch(() => {
                if (!cancelled) {
                    setFlowCatalog([])
                }
            })
        return () => {
            cancelled = true
        }
    }, [projectPath])

    const startNodeMissingFromOverride = Boolean(
        isInstalledFlowMode
        && draft.startNodeId
        && hydratedGraph
        && !hydratedGraph.nodes.some((node) => node.id === draft.startNodeId),
    )
    const continueDisabledReason = isLoadingPreview
        ? 'Loading the flow…'
        : hasValidationErrors
            ? 'Fix the flow’s validation errors before continuing.'
            : isInstalledFlowMode && !draft.overrideFlowName
                ? 'Pick a flow, or use the flow as this run used it.'
                : !draft.startNodeId
                    ? 'Pick the step to start from on the graph.'
                    : startNodeMissingFromOverride
                        ? `The flow as it is now has no step ${startNodeLabel ?? draft.startNodeId}.`
                        : isContinuing
                            ? 'Starting the new run…'
                            : undefined
    const canContinue = !continueDisabledReason
    const showValidationWarningBanner = diagnostics.some((diag) => diag.severity === 'warning') && !hasValidationErrors
    const canRetryLaunch = Boolean(draft.startNodeId) && !hasValidationErrors && !isContinuing

    const selectedLlmProvider = typeof graphAttrs.llm_provider === 'string' ? graphAttrs.llm_provider : ''
    const selectedLlmProfile = typeof graphAttrs.llm_profile === 'string' ? graphAttrs.llm_profile : ''
    const selectedReasoningEffort = typeof graphAttrs.reasoning_effort === 'string' ? graphAttrs.reasoning_effort : ''

    const requestContinue = async () => {
        if (!canContinue) {
            return
        }
        setContinueError(null)
        setIsContinuing(true)
        try {
            const result = await continueFromRun(
                draft.sourceRunId,
                {
                    projectPath: projectPath || draft.sourceWorkingDirectory,
                    workingDirectory: draft.workingDir,
                    model: draft.model.trim() || null,
                    llmProvider: selectedLlmProfile ? '' : selectedLlmProvider,
                    llmProfile: selectedLlmProfile,
                    reasoningEffort: selectedReasoningEffort,
                },
                {
                    startNodeId: draft.startNodeId || '',
                    flowSourceMode: draft.flowSourceMode,
                    flowName: isInstalledFlowMode ? draft.overrideFlowName : null,
                },
                { onGitPolicyWarning: setGitPolicyWarning },
            )
            if (result.status !== 'launched') {
                return
            }
            setLastLaunchFailure(null)
            onContinued(result.runId)
        } catch (error) {
            logUnexpectedLaunchError(error)
            const errorMessage = launchErrorMessage(error)
            setContinueError(errorMessage)
            setLastLaunchFailure({
                message: errorMessage,
                failedAt: new Date().toISOString(),
                flowSource: draft.sourceRunId,
            })
        } finally {
            setIsContinuing(false)
        }
    }

    return (
        <Well
            data-testid="run-continuation-panel"
            className="space-y-4"
        >
            <div className="flex items-start justify-between gap-3">
                <div className="space-y-1">
                    <h2 className="text-base font-semibold text-foreground">Continue from a step</h2>
                    <p data-testid="run-continuation-source-run" title={`From run ${draft.sourceRunId}`} className="text-xs leading-5 text-muted-foreground">
                        Starts a new run at the step you pick, with this run’s results so far. This run stays as it is.
                    </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                    <Button
                        type="button"
                        data-testid="run-continuation-continue-button"
                        onClick={() => {
                            void requestContinue()
                        }}
                        disabled={!canContinue}
                        title={canContinue ? undefined : continueDisabledReason}
                    >
                        Continue from step
                    </Button>
                    <Button
                        type="button"
                        data-testid="run-continuation-cancel-button"
                        variant="ghost"
                        size="sm"
                        onClick={onCancel}
                    >
                        Cancel
                    </Button>
                </div>
            </div>

            {previewLoadError ? (
                <InlineError data-testid="run-continuation-preview-error">{previewLoadError}</InlineError>
            ) : null}

            <LaunchNoticeStack
                showValidationWarningBanner={showValidationWarningBanner}
                runStartGitPolicyWarning={gitPolicyWarning}
                runStartError={continueError}
                lastLaunchFailure={lastLaunchFailure}
                canRetryLaunch={canRetryLaunch}
                onRetry={() => {
                    void requestContinue()
                }}
            />

            <div className="grid gap-4 md:grid-cols-2">
                <div className="space-y-2">
                    <p className="text-sm font-semibold text-foreground">
                        Use the flow
                    </p>
                    <SegmentedControl
                        aria-label="Use the flow"
                        className="w-fit"
                        options={[
                            {
                                key: 'snapshot',
                                label: 'As this run used it',
                                className: 'flex-none',
                                pressed: draft.flowSourceMode === 'snapshot',
                                'data-testid': 'run-continuation-use-snapshot-button',
                                onPress: () => onDraftChange({ flowSourceMode: 'snapshot', startNodeId: null }),
                            },
                            {
                                key: 'flow_name',
                                label: 'As it is now',
                                className: 'flex-none',
                                pressed: isInstalledFlowMode,
                                'data-testid': 'run-continuation-use-installed-flow-button',
                                onPress: () => onDraftChange({ flowSourceMode: 'flow_name', startNodeId: null }),
                            },
                        ]}
                    />
                    {isInstalledFlowMode ? (
                        <NativeSelect
                            data-testid="run-continuation-flow-override-select"
                            value={draft.overrideFlowName ?? ''}
                            onChange={(event) => {
                                onDraftChange({ overrideFlowName: event.target.value || null })
                            }}
                            size="sm"
                            className="w-full text-sm"
                        >
                            <NativeSelectOption value="">Pick a flow…</NativeSelectOption>
                            {flowCatalog.map((flow) => (
                                <NativeSelectOption key={flow} value={flow}>
                                    {flow}
                                </NativeSelectOption>
                            ))}
                        </NativeSelect>
                    ) : null}
                    <p data-testid="run-continuation-flow-source-copy" className="text-xs text-muted-foreground">
                        {draft.flowSourceMode === 'snapshot'
                            ? 'The new run uses the flow exactly as this run did.'
                            : draft.overrideFlowName
                                ? `The new run uses ${draft.overrideFlowName} as it is saved now, including any edits since this run started.`
                                : 'Pick the flow the new run uses.'}
                    </p>
                </div>
                <div className="space-y-4">
                    <div className="space-y-2">
                        <Label htmlFor="run-continuation-working-directory">Working directory</Label>
                        {/* Wraps so a long path shows whole; it is still one line, so Enter adds no break. */}
                        <Textarea
                            id="run-continuation-working-directory"
                            data-testid="run-continuation-working-directory-input"
                            rows={1}
                            spellCheck={false}
                            className="min-h-9 resize-none break-all py-1.5"
                            value={draft.workingDir}
                            onKeyDown={(event) => { if (event.key === 'Enter') event.preventDefault() }}
                            onChange={(event) => onDraftChange({ workingDir: event.target.value.replace(/[\r\n]+/g, '') })}
                        />
                    </div>
                    <div className="space-y-2">
                        <Label htmlFor="run-continuation-model">Model</Label>
                        <Input
                            id="run-continuation-model"
                            data-testid="run-continuation-model-input"
                            value={draft.model}
                            onChange={(event) => onDraftChange({ model: event.target.value })}
                            placeholder="The flow's own choice"
                        />
                    </div>
                </div>
            </div>

            <div
                data-testid="run-continuation-selected-node-copy"
                className="text-sm text-muted-foreground"
            >
                {draft.startNodeId
                    ? <>Starts at <span className="font-medium text-foreground" title={draft.startNodeId}>{startNodeLabel ?? draft.startNodeId}</span></>
                    : 'Pick the step to start from on the run graph.'}
                <p className="mt-1 text-xs text-muted-foreground">
                    The new run gets this run’s saved results, but not what earlier steps kept only in memory, such as an
                    agent’s conversation. A step that relied on that may fail or redo work.
                </p>
            </div>
        </Well>
    )
}
