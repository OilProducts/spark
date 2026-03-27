import { useState } from 'react'

import { Button } from '@/ui'
import type { RunRecord } from '../model/shared'
import {
    STATUS_LABELS,
    canCancelRun,
    cancelRunActionLabel,
    cancelRunDisabledReason,
    formatDuration,
    formatOutcomeLabel,
    formatTimestamp,
} from '../model/shared'
import { Panel, PanelContent, PanelHeader, SectionHeader } from '@/ui'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunSummaryCardProps {
    run: RunRecord
    activeProjectPath: string | null
    now: number
    onOpenRunArtifact: (run: RunRecord, artifactType: 'spec' | 'plan') => void
    onRequestCancel: (runId: string, currentStatus: string) => void
}

export function RunSummaryCard({ run, activeProjectPath, now, onOpenRunArtifact, onRequestCancel }: RunSummaryCardProps) {
    const cancelAvailable = canCancelRun(run.status)
    const [collapsed, setCollapsed] = useState(false)
    return (
        <Panel data-testid="run-summary-panel">
            <PanelHeader>
                <SectionHeader
                    title="Run Summary"
                    description="Execution metadata, scope, and final outcome."
                    action={(
                        <div className="flex items-center gap-2">
                            <span className="text-xs text-muted-foreground">{run.run_id}</span>
                            <RunSectionToggleButton
                                collapsed={collapsed}
                                onToggle={() => setCollapsed((current) => !current)}
                                testId="run-summary-toggle-button"
                            />
                        </div>
                    )}
                />
            </PanelHeader>
            {!collapsed ? (
                <PanelContent className="space-y-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                    <div className="flex flex-wrap gap-x-4 gap-y-2 text-sm">
                        {run.spec_id ? (
                            <Button
                                type="button"
                                data-testid="run-summary-spec-artifact-link"
                                onClick={() => onOpenRunArtifact(run, 'spec')}
                                variant="link"
                                size="xs"
                                className="h-auto px-0 py-0 font-mono"
                                title={run.spec_id}
                            >
                                Spec {run.spec_id}
                            </Button>
                        ) : null}
                        {run.plan_id ? (
                            <Button
                                type="button"
                                data-testid="run-summary-plan-artifact-link"
                                onClick={() => onOpenRunArtifact(run, 'plan')}
                                variant="link"
                                size="xs"
                                className="h-auto px-0 py-0 font-mono"
                                title={run.plan_id}
                            >
                                Plan {run.plan_id}
                            </Button>
                        ) : null}
                        </div>
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
                </PanelContent>
            ) : null}
        </Panel>
    )
}
