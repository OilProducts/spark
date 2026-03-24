import type { RunRecord } from '@/components/runs/shared'
import { STATUS_LABELS, formatDuration, formatOutcomeLabel, formatTimestamp } from '@/components/runs/shared'

interface RunSummaryCardProps {
    run: RunRecord
    activeProjectPath: string | null
    now: number
}

export function RunSummaryCard({ run, activeProjectPath, now }: RunSummaryCardProps) {
    return (
        <div data-testid="run-summary-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
            <div className="mb-3 flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Run Summary</h3>
                <span className="text-xs text-muted-foreground">{run.run_id}</span>
            </div>
            <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-2">
                <div data-testid="run-summary-status"><span className="font-medium">Status:</span> {STATUS_LABELS[run.status] || run.status}</div>
                <div data-testid="run-summary-outcome"><span className="font-medium">Outcome:</span> {formatOutcomeLabel(run.outcome)}</div>
                <div data-testid="run-summary-flow-name"><span className="font-medium">Flow:</span> {run.flow_name || 'Untitled'}</div>
                <div data-testid="run-summary-started-at"><span className="font-medium">Started:</span> {formatTimestamp(run.started_at)}</div>
                <div data-testid="run-summary-ended-at"><span className="font-medium">Ended:</span> {formatTimestamp(run.ended_at)}</div>
                <div data-testid="run-summary-duration"><span className="font-medium">Duration:</span> {formatDuration(run.started_at, run.ended_at, run.status, now)}</div>
                <div data-testid="run-summary-model"><span className="font-medium">Model:</span> {run.model || 'default model'}</div>
                <div data-testid="run-summary-working-directory" className="break-all"><span className="font-medium">Working Dir:</span> {run.working_directory || '—'}</div>
                <div data-testid="run-summary-project-path" className="break-all"><span className="font-medium">Project Path:</span> {run.project_path || activeProjectPath || '—'}</div>
                <div data-testid="run-summary-git-branch"><span className="font-medium">Git Branch:</span> {run.git_branch || '—'}</div>
                <div data-testid="run-summary-git-commit"><span className="font-medium">Git Commit:</span> {run.git_commit || '—'}</div>
                <div data-testid="run-summary-last-error" className="break-all"><span className="font-medium">Last Error:</span> {run.last_error || '—'}</div>
                <div data-testid="run-summary-token-usage"><span className="font-medium">Tokens:</span> {typeof run.token_usage === 'number' ? run.token_usage.toLocaleString() : '—'}</div>
            </div>
        </div>
    )
}
