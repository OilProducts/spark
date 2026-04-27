import type { ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import type { RunRecord } from '../model/shared'
import {
    STATUS_LABELS,
    canCancelRun,
    canContinueRun,
    canRetryRun,
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

function compactPath(value: string): string {
    return value.trim().replace(/\/+$/, '') || value.trim()
}

function shouldShowWorkingDirectoryDifference(run: RunRecord, activeProjectPath: string | null): boolean {
    const workingDirectory = run.working_directory ? compactPath(run.working_directory) : ''
    const projectPath = run.project_path || activeProjectPath || ''
    const compactProjectPath = projectPath ? compactPath(projectPath) : ''
    return Boolean(workingDirectory && compactProjectPath && workingDirectory !== compactProjectPath)
}

function formatGitRef(run: RunRecord): string | null {
    const branch = run.git_branch?.trim()
    const commit = run.git_commit?.trim()
    if (branch && commit) {
        return `${branch} @ ${commit.slice(0, 7)}`
    }
    return branch || (commit ? commit.slice(0, 7) : null)
}

function formatLineage(run: RunRecord): string | null {
    const lineageParts: string[] = []
    if (run.continued_from_run_id) {
        lineageParts.push(`Continued from ${run.continued_from_run_id}${run.continued_from_node ? ` @ ${run.continued_from_node}` : ''}`)
    }
    if (run.parent_run_id) {
        lineageParts.push(`Parent ${run.parent_run_id}${run.parent_node_id ? ` @ ${run.parent_node_id}` : ''}`)
    }
    if (run.root_run_id && run.root_run_id !== run.run_id) {
        lineageParts.push(`Root ${run.root_run_id}`)
    }
    if (run.child_invocation_index !== null && run.child_invocation_index !== undefined) {
        lineageParts.push(`Child invocation #${run.child_invocation_index}`)
    }
    return lineageParts.length > 0 ? lineageParts.join(' · ') : null
}

function formatOutcomeReason(run: RunRecord): string | null {
    const reasonMessage = run.outcome_reason_message?.trim()
    const reasonCode = run.outcome_reason_code?.trim()
    if (reasonMessage && reasonCode) {
        return `${reasonMessage} (${reasonCode})`
    }
    if (reasonMessage || reasonCode) {
        return reasonMessage || reasonCode || null
    }
    return run.last_error?.trim() || null
}

function SummarySection({
    children,
    testId,
    title,
}: {
    children: ReactNode
    testId: string
    title: string
}) {
    return (
        <section data-testid={testId} className="space-y-3 rounded-md border border-border/70 bg-muted/20 px-3 py-3">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h4>
            {children}
        </section>
    )
}

function SummaryRow({
    children,
    className = '',
    label,
    testId,
}: {
    children: ReactNode
    className?: string
    label: string
    testId: string
}) {
    return (
        <div data-testid={testId} className={`text-sm ${className}`}>
            <span className="font-medium text-foreground">{label}:</span> {children}
        </div>
    )
}

interface RunSummaryCardProps {
    run: RunRecord
    activeProjectPath: string | null
    now: number
    collapsed: boolean
    monitoringFacts: Array<{
        id: string
        label: string
        value: string
        testId: string
    }>
    monitoringHeadline: string
    onRequestCancel: (runId: string, currentStatus: string) => void
    onRequestRetry: (runId: string, currentStatus: string) => void
    onContinueFromRun: (run: RunRecord) => void
    onCollapsedChange: (collapsed: boolean) => void
}

export function RunSummaryCard({
    run,
    activeProjectPath,
    now,
    collapsed,
    monitoringFacts,
    monitoringHeadline,
    onRequestCancel,
    onRequestRetry,
    onContinueFromRun,
    onCollapsedChange,
}: RunSummaryCardProps) {
    const cancelAvailable = canCancelRun(run.status)
    const continueAvailable = canContinueRun(run.status)
    const retryAvailable = canRetryRun(run.status)
    const usageBreakdown = run.token_usage_breakdown
    const modelUsageEntries = Object.entries(usageBreakdown?.by_model ?? {})
    const projectPath = run.project_path || activeProjectPath || '—'
    const gitRef = formatGitRef(run)
    const lineage = formatLineage(run)
    const outcomeReason = formatOutcomeReason(run)
    const costNote = formatEstimatedModelCostNote(run)
    const showWorkingDirectoryDifference = shouldShowWorkingDirectoryDifference(run, activeProjectPath)
    return (
        <Card data-testid="run-summary-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Run Summary</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Current activity, outcome, scope, and usage.
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
                    <div className="flex flex-wrap justify-end gap-2">
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
                        {retryAvailable ? (
                            <Button
                                type="button"
                                data-testid="run-summary-retry-button"
                                onClick={() => onRequestRetry(run.run_id, run.status)}
                                variant="secondary"
                                size="xs"
                            >
                                Retry run
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
                    <div className="grid gap-4 xl:grid-cols-2">
                        <SummarySection testId="run-summary-section-now" title="Now">
                            <div className="flex flex-wrap items-center justify-between gap-2">
                                <div data-testid="run-activity-headline" className="text-sm font-medium text-foreground">
                                    {monitoringHeadline}
                                </div>
                                <div data-testid="run-activity-status" className="text-xs text-muted-foreground">
                                    {STATUS_LABELS[run.status] || run.status}
                                </div>
                            </div>
                            <div data-testid="run-activity-panel" className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
                                {monitoringFacts.map((fact) => (
                                    <div key={fact.id} data-testid={fact.testId}>
                                        <span className="font-medium text-foreground">{fact.label}:</span> {fact.value}
                                    </div>
                                ))}
                            </div>
                        </SummarySection>
                        <SummarySection testId="run-summary-section-outcome" title="Outcome">
                            <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                                <SummaryRow testId="run-summary-status" label="Status">
                                    {STATUS_LABELS[run.status] || run.status}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-outcome" label="Outcome">
                                    {formatOutcomeLabel(run.outcome)}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-duration" label="Duration">
                                    {formatDuration(run.started_at, run.ended_at, run.status, now)}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-started-at" label="Started">
                                    {formatTimestamp(run.started_at)}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-ended-at" label="Ended">
                                    {formatTimestamp(run.ended_at)}
                                </SummaryRow>
                                {outcomeReason ? (
                                    <SummaryRow testId="run-summary-outcome-reason" label="Reason" className="break-all md:col-span-2">
                                        {outcomeReason}
                                    </SummaryRow>
                                ) : null}
                            </div>
                        </SummarySection>
                        <SummarySection testId="run-summary-section-scope" title="Scope">
                            <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                                <SummaryRow testId="run-summary-flow-name" label="Flow">
                                    {run.flow_name || 'Untitled'}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-project-path" label="Project" className="break-all">
                                    {projectPath}
                                </SummaryRow>
                                {gitRef ? (
                                    <SummaryRow testId="run-summary-git-ref" label="Git">
                                        {gitRef}
                                    </SummaryRow>
                                ) : null}
                                {run.spec_id || run.plan_id ? (
                                    <SummaryRow testId="run-summary-artifacts" label="Artifacts" className="md:col-span-2">
                                        <span className="inline-flex flex-wrap gap-2">
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
                                        </span>
                                    </SummaryRow>
                                ) : null}
                                {lineage ? (
                                    <SummaryRow testId="run-summary-lineage" label="Lineage" className="md:col-span-2">
                                        {lineage}
                                    </SummaryRow>
                                ) : null}
                                {showWorkingDirectoryDifference ? (
                                    <SummaryRow testId="run-summary-working-directory-note" label="Working dir differs" className="break-all md:col-span-2">
                                        {run.working_directory}
                                    </SummaryRow>
                                ) : null}
                            </div>
                        </SummarySection>
                        <SummarySection testId="run-summary-section-usage" title="Usage">
                            <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                                <SummaryRow testId="run-summary-token-usage" label="Total tokens">
                                    {formatTokenCount(usageBreakdown?.total_tokens ?? run.token_usage)}
                                </SummaryRow>
                                <SummaryRow testId="run-summary-estimated-model-cost" label="Estimated cost">
                                    {formatEstimatedModelCostLabel(run)}
                                </SummaryRow>
                                {usageBreakdown ? (
                                    <>
                                        <SummaryRow testId="run-summary-input-tokens" label="Input tokens">
                                            {formatTokenCount(usageBreakdown.input_tokens)}
                                        </SummaryRow>
                                        <SummaryRow testId="run-summary-cached-input-tokens" label="Cached input tokens">
                                            {formatTokenCount(usageBreakdown.cached_input_tokens)}
                                        </SummaryRow>
                                        <SummaryRow testId="run-summary-output-tokens" label="Output tokens">
                                            {formatTokenCount(usageBreakdown.output_tokens)}
                                        </SummaryRow>
                                    </>
                                ) : null}
                            </div>
                            {costNote ? (
                                <div data-testid="run-summary-estimated-model-cost-note" className="text-xs text-muted-foreground">
                                    {costNote}
                                </div>
                            ) : null}
                            {modelUsageEntries.length > 0 ? (
                                <div data-testid="run-summary-model-breakdown" className="space-y-2 rounded-md border border-border/70 bg-background/70 p-3">
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
                                                    className="rounded-sm border border-border/70 bg-card px-3 py-2 text-sm"
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
                        </SummarySection>
                    </div>
                </CardContent>
            ) : null}
        </Card>
    )
}
