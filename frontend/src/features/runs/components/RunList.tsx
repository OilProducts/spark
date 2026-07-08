import { cn } from '@/lib/utils'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { formatProjectPathLabel } from '@/lib/projectPaths'
import type { RunRecord } from '../model/shared'
import {
    STATUS_LABELS,
    STATUS_STYLES,
    formatDuration,
} from '../model/shared'

const ACTIVE_LIST_STATUSES = new Set([
    'running',
    'queued',
    'pause_requested',
    'abort_requested',
    'cancel_requested',
])

interface RunListProps {
    activeProjectPath: string | null
    error: string | null
    scopeMode: 'active' | 'all'
    onScopeModeChange: (mode: 'active' | 'all') => void
    status: 'idle' | 'loading' | 'ready' | 'error'
    onSelectRun: (run: RunRecord) => void
    runs: RunRecord[]
    selectedRunId: string | null
    summaryLabel: string
}

export function RunList({
    activeProjectPath,
    error,
    scopeMode,
    onScopeModeChange,
    status,
    onSelectRun,
    runs,
    selectedRunId,
    summaryLabel,
}: RunListProps) {
    const isNarrowViewport = useNarrowViewport()
    const scopeDescription = scopeMode === 'all'
        ? 'Run history across all projects.'
        : activeProjectPath
            ? 'Run history for the active project.'
            : 'Choose an active project or switch to all projects.'
    const activeProjectLabel = activeProjectPath
        ? formatProjectPathLabel(activeProjectPath)
        : 'No active project'
    const compactProjectLabel = (projectPath?: string | null) => {
        return projectPath ? formatProjectPathLabel(projectPath) : null
    }
    const queuedLockGroups = runs.reduce<Array<{ identity: string; label: string; runs: RunRecord[] }>>((groups, run) => {
        const executionLock = run.execution_lock
        if (run.status !== 'queued' || !executionLock?.identity) {
            return groups
        }
        const label = `${executionLock.scope} lock · ${executionLock.key}`
        const existingGroup = groups.find((group) => group.identity === executionLock.identity)
        if (existingGroup) {
            existingGroup.runs.push(run)
            return groups
        }
        return [...groups, { identity: executionLock.identity, label, runs: [run] }]
    }, [])
    const historyRuns = runs.filter((run) => run.status !== 'queued' || !run.execution_lock?.identity)

    // Work-queue grouping: children nest under their parent; parentless (or
    // orphaned) runs bucket by how actionable they are.
    const listedRunIds = new Set(historyRuns.map((run) => run.run_id))
    const childRunsByParent = new Map<string, RunRecord[]>()
    const topLevelRuns: RunRecord[] = []
    for (const run of historyRuns) {
        if (run.parent_run_id && listedRunIds.has(run.parent_run_id)) {
            const siblings = childRunsByParent.get(run.parent_run_id) ?? []
            siblings.push(run)
            childRunsByParent.set(run.parent_run_id, siblings)
        } else {
            topLevelRuns.push(run)
        }
    }
    const needsInputRuns = topLevelRuns.filter((run) => run.status === 'waiting')
    const runningRuns = topLevelRuns.filter((run) => ACTIVE_LIST_STATUSES.has(run.status))
    const recentRuns = topLevelRuns.filter(
        (run) => run.status !== 'waiting' && !ACTIVE_LIST_STATUSES.has(run.status),
    )

    const renderRunRow = (run: RunRecord, depth = 0) => {
        const shortRunId = run.run_id.slice(0, 8)
        const projectLabel = scopeMode === 'all' ? compactProjectLabel(run.project_path) : null
        const metaParts = [
            formatDuration(run.started_at, run.ended_at, run.status),
            shortRunId,
        ].filter((value) => Boolean(value) && value !== '—')
        const holdsExecutionLock = run.execution_lock?.state === 'holding'
        const queuedForExecutionLock = run.execution_lock?.state === 'queued'

        const childRuns = childRunsByParent.get(run.run_id) ?? []

        return (
            <div key={run.run_id} className={cn(depth > 0 && 'ml-4 border-l border-border/60 pl-2')}>
            <article
                data-testid="run-history-row"
                role="button"
                tabIndex={0}
                aria-pressed={selectedRunId === run.run_id}
                onClick={() => onSelectRun(run)}
                onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault()
                        onSelectRun(run)
                    }
                }}
                className={cn(
                    'rounded-lg border border-border/80 bg-card/80 px-3 py-2.5 shadow-sm outline-none transition-colors hover:border-primary/40 focus-visible:ring-2 focus-visible:ring-primary/30 cursor-pointer',
                    selectedRunId === run.run_id && 'border-primary/50 bg-muted/30 ring-1 ring-primary/20',
                )}
            >
                <div className="space-y-2">
                    <div className="flex items-start gap-3">
                        <div className="min-w-0 flex-1 space-y-1">
                            <div className="truncate text-sm font-medium text-foreground" title={run.flow_name || run.run_id}>
                                {run.flow_name || run.run_id.slice(0, 8)}
                            </div>
                            <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
                                {metaParts.length > 0 ? (
                                    <span className="truncate">{metaParts.join(' · ')}</span>
                                ) : null}
                                {projectLabel ? (
                                    <span className="truncate" title={run.project_path}>
                                        {projectLabel}
                                    </span>
                                ) : null}
                                {run.root_run_id && run.root_run_id !== run.run_id ? (
                                    <span className="truncate" title={run.root_run_id}>
                                        root {run.root_run_id.slice(0, 8)}
                                    </span>
                                ) : null}
                            </div>
                            {holdsExecutionLock ? (
                                <div className="text-[11px] font-medium text-amber-800">
                                    Holding execution lock
                                </div>
                            ) : null}
                            {queuedForExecutionLock ? (
                                <div className="text-[11px] font-medium text-amber-800">
                                    Queued for execution lock{typeof run.execution_lock?.queue_position === 'number'
                                        ? ` · position ${run.execution_lock.queue_position}`
                                        : ''}
                                </div>
                            ) : null}
                        </div>
                        <div className="flex shrink-0 flex-wrap items-center gap-2">
                            {run.parent_run_id ? (
                                <span className="inline-flex h-6 items-center justify-center rounded-md border border-border/70 px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                    Child
                                </span>
                            ) : null}
                            <span
                                className={`inline-flex h-6 items-center justify-center rounded-md px-2 text-[11px] font-semibold uppercase tracking-wide ${
                                    STATUS_STYLES[run.status] || 'bg-muted text-muted-foreground'
                                }`}
                            >
                                {STATUS_LABELS[run.status] || run.status}
                            </span>
                        </div>
                    </div>
                </div>
            </article>
            {childRuns.length > 0 ? (
                <div data-testid="run-history-children" className="mt-2 space-y-2">
                    {childRuns.map((child) => renderRunRow(child, depth + 1))}
                </div>
            ) : null}
            </div>
        )
    }

    const renderRunGroup = (
        key: string,
        label: string,
        groupRuns: RunRecord[],
        accent?: string,
    ) => {
        if (groupRuns.length === 0) {
            return null
        }
        return (
            <section key={key} data-testid={`run-list-group-${key}`} className="space-y-2">
                <div className={cn(
                    'flex items-center justify-between px-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground',
                    accent,
                )}
                >
                    <span>{label}</span>
                    <span data-testid={`run-list-group-${key}-count`}>{groupRuns.length}</span>
                </div>
                <div className="space-y-3">
                    {groupRuns.map((run) => renderRunRow(run))}
                </div>
            </section>
        )
    }

    return (
        <nav
            data-testid="run-list-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`bg-background flex shrink-0 flex-col overflow-hidden z-40 ${
                isNarrowViewport ? 'w-full max-h-[46vh] rounded-md border' : 'w-80 border-r'
            }`}
        >
            <div className="px-4 pb-2 pt-4">
                <div className="flex items-center gap-3 text-xs font-semibold uppercase tracking-[0.2em] text-foreground">
                    <span>Runs</span>
                    <span className="h-2 w-2 rounded-full bg-muted-foreground/40" />
                </div>
                <Badge
                    data-testid="runs-project-context-chip"
                    variant="outline"
                    className="mt-3"
                    title={activeProjectPath || 'No active project'}
                >
                    <span className="text-muted-foreground">Project:</span>
                    <span className="max-w-40 truncate">{activeProjectLabel}</span>
                </Badge>
            </div>
            <div className="space-y-3 px-4 pb-3">
                <div className="space-y-1">
                    <h2 className="text-sm font-semibold tracking-tight">Run History</h2>
                    <p className="text-xs text-muted-foreground">{summaryLabel}</p>
                    <p className="text-xs text-muted-foreground">{scopeDescription}</p>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                    <Button
                        type="button"
                        data-testid="runs-scope-active-project"
                        onClick={() => onScopeModeChange('active')}
                        variant={scopeMode === 'active' ? 'secondary' : 'outline'}
                        size="xs"
                        disabled={!activeProjectPath}
                    >
                        Active project
                    </Button>
                    <Button
                        type="button"
                        data-testid="runs-scope-all-projects"
                        onClick={() => onScopeModeChange('all')}
                        variant={scopeMode === 'all' ? 'secondary' : 'outline'}
                        size="xs"
                    >
                        All projects
                    </Button>
                </div>
                {error ? (
                    <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                        <AlertDescription className="text-inherit">{error}</AlertDescription>
                    </Alert>
                ) : null}
                {scopeMode === 'active' && !activeProjectPath ? (
                    <Alert className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground">
                        <AlertDescription className="text-inherit">
                            Choose an active project or switch to all projects to view run history.
                        </AlertDescription>
                    </Alert>
                ) : null}
            </div>
            {status !== 'ready' && status !== 'error' && runs.length === 0 ? (
                <div className="px-4 pb-4">
                    <Alert
                        data-testid="run-list-loading"
                        className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                    >
                        <AlertDescription className="text-inherit">
                            Restoring run history…
                        </AlertDescription>
                    </Alert>
                </div>
            ) : runs.length === 0 ? (
                <div className="px-4 pb-4">
                    <Empty className="px-3 py-4 text-xs text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>
                                {scopeMode === 'all'
                                    ? 'No runs yet.'
                                    : activeProjectPath
                                        ? 'No runs for the active project yet.'
                                        : 'Choose an active project or switch to all projects.'}
                            </EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                </div>
            ) : (
                <div
                    data-testid="run-list-scroll-region"
                    className="min-h-0 flex-1 overflow-y-auto px-3 pb-4"
                >
                    <div className="space-y-4">
                        {renderRunGroup('needs-input', 'Needs input', needsInputRuns, 'text-sky-700')}
                        {renderRunGroup('running', 'Running', runningRuns)}
                        {queuedLockGroups.map((group) => (
                            <section key={group.identity} className="space-y-2">
                                <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] font-medium text-amber-800">
                                    Queued execution lock · {group.label}
                                </div>
                                <div className="space-y-3">
                                    {group.runs.map((run) => renderRunRow(run))}
                                </div>
                            </section>
                        ))}
                        {renderRunGroup('recent', 'Recent', recentRuns)}
                    </div>
                </div>
            )}
        </nav>
    )
}
