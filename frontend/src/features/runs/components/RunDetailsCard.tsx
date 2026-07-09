import type { ReactNode } from 'react'
import type { RunRecord } from '../model/shared'
import { STATUS_LABELS, formatDuration, formatOutcomeLabel, formatTimestamp } from '../model/shared'
import {
    formatEstimatedCost,
    formatOutcomeReason,
    formatEstimatedModelCostLabel,
    formatEstimatedModelCostNote,
    formatGitRef,
    formatLineage,
    formatTokenCount,
    hasExecutionLockMetadata,
    hasExecutionMetadata,
    shouldShowWorkingDirectoryDifference,
} from '../model/runSummaryFormat'

// Reference detail for a run — scope, execution, lock, and usage — behind the
// inspector's Details tab. Ambient monitoring lives in the header bar.

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

export interface RunDetailsCardProps {
    run: RunRecord
    activeProjectPath: string | null
    now: number
    /** Node the checkpoint would resume from, for Continue/Retry decisions. */
    resumeNode?: string | null
}

export function RunDetailsCard({ run, activeProjectPath, now, resumeNode = null }: RunDetailsCardProps) {
    const usageBreakdown = run.token_usage_breakdown
    const modelUsageEntries = Object.entries(usageBreakdown?.by_model ?? {})
    const projectPath = run.project_path || activeProjectPath || '—'
    const gitRef = formatGitRef(run)
    const lineage = formatLineage(run)
    const costNote = formatEstimatedModelCostNote(run)
    const showWorkingDirectoryDifference = shouldShowWorkingDirectoryDifference(run, activeProjectPath)

    const outcomeReason = formatOutcomeReason(run)
    return (
        <div data-testid="run-details-card" className="space-y-4">
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
                    {resumeNode ? (
                        <SummaryRow testId="run-summary-resume-node" label="Resumes from">
                            {resumeNode}
                        </SummaryRow>
                    ) : null}
                </div>
            </SummarySection>
            <SummarySection testId="run-summary-section-scope" title="Scope">
                <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                    <SummaryRow testId="run-summary-flow-name" label="Flow">
                        {run.flow_name || run.run_id.slice(0, 8)}
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
            {hasExecutionMetadata(run) ? (
                <SummarySection testId="run-summary-section-execution" title="Execution">
                    <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                        {run.execution_profile_id ? (
                            <SummaryRow testId="run-summary-execution-profile" label="Profile">
                                {run.execution_profile_id}
                            </SummaryRow>
                        ) : null}
                        {run.execution_mode ? (
                            <SummaryRow testId="run-summary-execution-mode" label="Mode">
                                {run.execution_mode}
                            </SummaryRow>
                        ) : null}
                        {run.execution_container_image ? (
                            <SummaryRow testId="run-summary-execution-container-image" label="Container image" className="break-all">
                                {run.execution_container_image}
                            </SummaryRow>
                        ) : null}
                    </div>
                </SummarySection>
            ) : null}
            {hasExecutionLockMetadata(run) ? (
                <SummarySection testId="run-summary-section-execution-lock" title="Execution Lock">
                    <div className="grid gap-x-4 gap-y-2 md:grid-cols-2">
                        <SummaryRow testId="run-summary-execution-lock-state" label="State">
                            {run.execution_lock?.state === 'holding'
                                ? 'Holding execution lock'
                                : run.execution_lock?.state === 'queued'
                                    ? 'Queued for execution lock'
                                    : run.execution_lock?.state || '—'}
                        </SummaryRow>
                        <SummaryRow testId="run-summary-execution-lock-key" label="Key">
                            {run.execution_lock?.key || '—'}
                        </SummaryRow>
                        <SummaryRow testId="run-summary-execution-lock-scope" label="Scope">
                            {run.execution_lock?.scope || '—'}
                        </SummaryRow>
                        <SummaryRow testId="run-summary-execution-lock-conflict-policy" label="Conflict policy">
                            {run.execution_lock?.conflict_policy || '—'}
                        </SummaryRow>
                        {typeof run.execution_lock?.queue_position === 'number' ? (
                            <SummaryRow testId="run-summary-execution-lock-queue-position" label="Queue position">
                                {run.execution_lock.queue_position}
                            </SummaryRow>
                        ) : null}
                    </div>
                </SummarySection>
            ) : null}
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
    )
}
