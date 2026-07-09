import { useEffect, useState } from 'react'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
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
    activeProjectPath: string | null
    onDraftChange: (patch: Partial<ContinuationDraft>) => void
    onCancel: () => void
    onContinued: (runId: string | null) => void
}

export function RunContinuationPanel({
    draft,
    activeProjectPath,
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
    } = useLaunchPreview(previewSource)

    useEffect(() => {
        let cancelled = false
        loadFlowCatalog()
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
    }, [])

    const startNodeMissingFromOverride = Boolean(
        isInstalledFlowMode
        && draft.startNodeId
        && hydratedGraph
        && !hydratedGraph.nodes.some((node) => node.id === draft.startNodeId),
    )
    const continueDisabledReason = isLoadingPreview
        ? 'Loading graph preview for continuation.'
        : hasValidationErrors
            ? 'Fix validation errors before continuing.'
            : isInstalledFlowMode && !draft.overrideFlowName
                ? 'Select an installed flow override or switch back to the source snapshot.'
                : !draft.startNodeId
                    ? 'Select a restart node in the graph.'
                    : startNodeMissingFromOverride
                        ? `Node ${draft.startNodeId} does not exist in the installed flow.`
                        : isContinuing
                            ? 'Continuation in progress.'
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
                    projectPath: activeProjectPath || draft.sourceWorkingDirectory,
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
            if (result.status === 'cancelled') {
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
        <div
            data-testid="run-continuation-panel"
            className="space-y-4 rounded-lg border border-border/80 bg-muted/10 p-4"
        >
            <div className="flex items-start justify-between gap-3">
                <div className="space-y-1">
                    <h3 className="text-sm font-semibold text-foreground">Continue Run</h3>
                    <p className="text-xs leading-5 text-muted-foreground">
                        Create a derived run from <span className="font-mono" data-testid="run-continuation-source-run">{draft.sourceRunId}</span> using inherited checkpoint context.
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
                        Continue from node
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
                <Alert
                    data-testid="run-continuation-preview-error"
                    className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                >
                    <AlertDescription className="text-inherit">{previewLoadError}</AlertDescription>
                </Alert>
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
                    <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        Graph source
                    </p>
                    <div className="flex flex-wrap gap-2">
                        <Button
                            type="button"
                            size="xs"
                            variant={draft.flowSourceMode === 'snapshot' ? 'secondary' : 'outline'}
                            data-testid="run-continuation-use-snapshot-button"
                            onClick={() => {
                                onDraftChange({ flowSourceMode: 'snapshot', startNodeId: null })
                            }}
                        >
                            Use source snapshot
                        </Button>
                        <Button
                            type="button"
                            size="xs"
                            variant={isInstalledFlowMode ? 'secondary' : 'outline'}
                            data-testid="run-continuation-use-installed-flow-button"
                            onClick={() => {
                                onDraftChange({ flowSourceMode: 'flow_name', startNodeId: null })
                            }}
                        >
                            Use installed flow
                        </Button>
                    </div>
                    {isInstalledFlowMode ? (
                        <NativeSelect
                            data-testid="run-continuation-flow-override-select"
                            value={draft.overrideFlowName ?? ''}
                            onChange={(event) => {
                                onDraftChange({ overrideFlowName: event.target.value || null })
                            }}
                            size="sm"
                            className="w-full text-xs"
                        >
                            <NativeSelectOption value="">Select an installed flow…</NativeSelectOption>
                            {flowCatalog.map((flow) => (
                                <NativeSelectOption key={flow} value={flow}>
                                    {flow}
                                </NativeSelectOption>
                            ))}
                        </NativeSelect>
                    ) : null}
                    <p data-testid="run-continuation-flow-source-copy" className="text-xs text-muted-foreground">
                        {draft.flowSourceMode === 'snapshot'
                            ? 'Continuing from the stored source-run graph snapshot.'
                            : draft.overrideFlowName
                                ? `Continuing with installed flow override ${draft.overrideFlowName}.`
                                : 'Select an installed flow to continue with an override.'}
                    </p>
                </div>
                <div className="space-y-4">
                    <div className="space-y-2">
                        <Label htmlFor="run-continuation-working-directory">Working directory</Label>
                        <Input
                            id="run-continuation-working-directory"
                            data-testid="run-continuation-working-directory-input"
                            value={draft.workingDir}
                            onChange={(event) => onDraftChange({ workingDir: event.target.value })}
                        />
                    </div>
                    <div className="space-y-2">
                        <Label htmlFor="run-continuation-model">Model override</Label>
                        <Input
                            id="run-continuation-model"
                            data-testid="run-continuation-model-input"
                            value={draft.model}
                            onChange={(event) => onDraftChange({ model: event.target.value })}
                            placeholder="Use server default"
                        />
                    </div>
                </div>
            </div>

            <div
                data-testid="run-continuation-selected-node-copy"
                className="rounded-md border border-border/80 bg-background/80 px-3 py-2 text-sm text-muted-foreground"
            >
                {draft.startNodeId
                    ? <>Restart node: <span className="font-mono text-foreground">{draft.startNodeId}</span></>
                    : 'Select a restart node in the run graph.'}
                <p className="mt-1 text-xs text-muted-foreground">
                    The derived run inherits the source checkpoint context; nodes may fail when restarted cold.
                </p>
            </div>
        </div>
    )
}
