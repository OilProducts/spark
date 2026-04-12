import { Button } from '@/components/ui/button'
import type { RunRecord } from '../model/shared'
import {
    STATUS_LABELS,
    canCancelRun,
    canContinueRun,
    cancelRunActionLabel,
    cancelRunDisabledReason,
    formatDuration,
    formatOutcomeLabel,
    formatTimestamp,
} from '../model/shared'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { RunSectionToggleButton } from './RunSectionToggleButton'

function formatTokenCount(value: number | null | undefined): string {
    return typeof value === 'number' ? value.toLocaleString() : '—'
}

function formatEstimatedCost(amount: number | null | undefined, currency: string): string {
    if (typeof amount !== 'number') {
        return 'Unpriced'
    }
    const maximumFractionDigits = amount >= 1 ? 2 : 6
    const minimumFractionDigits = amount >= 1 ? 2 : 4
    return new Intl.NumberFormat('en-US', {
        style: 'currency',
        currency,
        minimumFractionDigits,
        maximumFractionDigits,
    }).format(amount)
}

function formatEstimatedModelCostLabel(run: RunRecord): string {
    const estimatedCost = run.estimated_model_cost
    if (!estimatedCost) {
        return '—'
    }
    if (estimatedCost.status === 'unpriced') {
        return 'Unpriced model usage'
    }
    const prefix = formatEstimatedCost(estimatedCost.amount, estimatedCost.currency)
    if (estimatedCost.status === 'partial_unpriced') {
        return `${prefix} (partial)`
    }
    return prefix
}

function formatEstimatedModelCostNote(run: RunRecord): string | null {
    const estimatedCost = run.estimated_model_cost
    if (!estimatedCost || estimatedCost.unpriced_models.length === 0) {
        return null
    }
    const label = estimatedCost.status === 'partial_unpriced'
        ? 'Unpriced models excluded from the subtotal'
        : 'Unpriced models'
    return `${label}: ${estimatedCost.unpriced_models.join(', ')}`
}

interface RunSummaryCardProps {
    run: RunRecord
    activeProjectPath: string | null
    now: number
    collapsed: boolean
    onRequestCancel: (runId: string, currentStatus: string) => void
    onContinueFromRun: (run: RunRecord) => void
    onCollapsedChange: (collapsed: boolean) => void
}

export function RunSummaryCard({
    run,
    activeProjectPath,
    now,
    collapsed,
    onRequestCancel,
    onContinueFromRun,
    onCollapsedChange,
}: RunSummaryCardProps) {
    const cancelAvailable = canCancelRun(run.status)
    const continueAvailable = canContinueRun(run.status)
    const usageBreakdown = run.token_usage_breakdown
    const modelUsageEntries = Object.entries(usageBreakdown?.by_model ?? {})
    return (
        <Card data-testid="run-summary-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Run Summary</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Execution metadata, scope, and final outcome.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <span className="text-xs text-muted-foreground">{run.run_id}</span>
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-summary-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-4 px-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                    <div className="flex flex-wrap gap-x-4 gap-y-2 text-sm">
                        {run.spec_id ? (
                            <span
                                data-testid="run-summary-spec-artifact-link"
                                className="font-mono text-xs text-muted-foreground"
                                title={run.spec_id}
                            >
                                Spec {run.spec_id}
                            </span>
                        ) : null}
                        {run.plan_id ? (
                            <span
                                data-testid="run-summary-plan-artifact-link"
                                className="font-mono text-xs text-muted-foreground"
                                title={run.plan_id}
                            >
                                Plan {run.plan_id}
                            </span>
                        ) : null}
                        </div>
                        <div className="flex items-center gap-2">
                            {continueAvailable ? (
                                <Button
                                    type="button"
                                    data-testid="run-summary-continue-button"
                                    onClick={() => onContinueFromRun(run)}
                                    variant="outline"
                                    size="xs"
                                >
                                    Continue from node
                                </Button>
                            ) : null}
                            <Button
                                type="button"
                                data-testid="run-summary-cancel-button"
                                onClick={() => onRequestCancel(run.run_id, run.status)}
                                disabled={!cancelAvailable}
                                title={cancelAvailable ? undefined : cancelRunDisabledReason(run.status)}
                                variant={cancelAvailable ? 'destructive' : 'outline'}
                                size="xs"
                            >
                                {cancelRunActionLabel(run.status)}
                            </Button>
                        </div>
                </div>
                <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-2">
                <div data-testid="run-summary-status"><span className="font-medium">Status:</span> {STATUS_LABELS[run.status] || run.status}</div>
                <div data-testid="run-summary-outcome"><span className="font-medium">Outcome:</span> {formatOutcomeLabel(run.outcome)}</div>
                <div data-testid="run-summary-flow-name"><span className="font-medium">Flow:</span> {run.flow_name || 'Untitled'}</div>
                <div data-testid="run-summary-started-at"><span className="font-medium">Started:</span> {formatTimestamp(run.started_at)}</div>
                <div data-testid="run-summary-ended-at"><span className="font-medium">Ended:</span> {formatTimestamp(run.ended_at)}</div>
                <div data-testid="run-summary-duration"><span className="font-medium">Duration:</span> {formatDuration(run.started_at, run.ended_at, run.status, now)}</div>
                <div data-testid="run-summary-model"><span className="font-medium">Launch model:</span> {run.model || 'default model'}</div>
                <div data-testid="run-summary-working-directory" className="break-all"><span className="font-medium">Working Dir:</span> {run.working_directory || '—'}</div>
                <div data-testid="run-summary-project-path" className="break-all"><span className="font-medium">Project Path:</span> {run.project_path || activeProjectPath || '—'}</div>
                <div data-testid="run-summary-git-branch"><span className="font-medium">Git Branch:</span> {run.git_branch || '—'}</div>
                <div data-testid="run-summary-git-commit"><span className="font-medium">Git Commit:</span> {run.git_commit || '—'}</div>
                <div data-testid="run-summary-continued-from"><span className="font-medium">Continued From:</span> {run.continued_from_run_id ? `${run.continued_from_run_id} @ ${run.continued_from_node || '—'}` : '—'}</div>
                <div data-testid="run-summary-last-error" className="break-all"><span className="font-medium">Last Error:</span> {run.last_error || '—'}</div>
                <div data-testid="run-summary-estimated-model-cost"><span className="font-medium">Estimated model cost:</span> {formatEstimatedModelCostLabel(run)}</div>
                <div data-testid="run-summary-token-usage"><span className="font-medium">Total tokens:</span> {formatTokenCount(usageBreakdown?.total_tokens ?? run.token_usage)}</div>
                <div data-testid="run-summary-input-tokens"><span className="font-medium">Input tokens:</span> {formatTokenCount(usageBreakdown?.input_tokens)}</div>
                <div data-testid="run-summary-cached-input-tokens"><span className="font-medium">Cached input tokens:</span> {formatTokenCount(usageBreakdown?.cached_input_tokens)}</div>
                <div data-testid="run-summary-output-tokens"><span className="font-medium">Output tokens:</span> {formatTokenCount(usageBreakdown?.output_tokens)}</div>
                </div>
                {formatEstimatedModelCostNote(run) ? (
                    <div data-testid="run-summary-estimated-model-cost-note" className="text-xs text-muted-foreground">
                        {formatEstimatedModelCostNote(run)}
                    </div>
                ) : null}
                {modelUsageEntries.length > 0 ? (
                    <div data-testid="run-summary-model-breakdown" className="space-y-2 rounded-md border border-border/70 bg-muted/20 p-3">
                        <div className="text-sm font-medium">Per-model breakdown</div>
                        <div className="space-y-2">
                            {modelUsageEntries.map(([modelId, usage]) => {
                                const modelCost = run.estimated_model_cost?.by_model?.[modelId]
                                const modelCostLabel = modelCost?.status === 'estimated'
                                    ? formatEstimatedCost(modelCost.amount, modelCost.currency)
                                    : 'Unpriced'
                                return (
                                    <div
                                        key={modelId}
                                        data-testid="run-summary-model-row"
                                        className="rounded-sm border border-border/70 bg-background/70 px-3 py-2 text-sm"
                                    >
                                        <div className="font-mono text-xs text-muted-foreground">{modelId}</div>
                                        <div className="mt-1 grid gap-x-4 gap-y-1 md:grid-cols-5">
                                            <div><span className="font-medium">Input:</span> {formatTokenCount(usage.input_tokens)}</div>
                                            <div><span className="font-medium">Cached:</span> {formatTokenCount(usage.cached_input_tokens)}</div>
                                            <div><span className="font-medium">Output:</span> {formatTokenCount(usage.output_tokens)}</div>
                                            <div><span className="font-medium">Total:</span> {formatTokenCount(usage.total_tokens)}</div>
                                            <div><span className="font-medium">Cost:</span> {modelCostLabel}</div>
                                        </div>
                                    </div>
                                )
                            })}
                        </div>
                    </div>
                ) : null}
                </CardContent>
            ) : null}
        </Card>
    )
}
