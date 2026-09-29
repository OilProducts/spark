import { useState } from 'react'

import { cn } from '@/lib/utils'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { InlineError } from '@/components/app/inline-error'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { formatProjectPathLabel } from '@/lib/projectPaths'
import type { RunRecord } from '../model/shared'
import { formatDuration } from '../model/shared'
import { flowTitle, formatRunAge, runTitle } from '../model/runOverviewModel'

const ACTIVE_LIST_STATUSES = new Set([
    'running',
    'queued',
    'pause_requested',
    'abort_requested',
    'cancel_requested',
])

// A status word only when the run needs attention.
const ATTENTION: Record<string, { label: string; className: string }> = {
    failed: { label: 'Failed', className: 'text-destructive' },
    validation_error: { label: 'Failed', className: 'text-destructive' },
    waiting: { label: 'Needs input', className: 'text-warning' },
    running: { label: 'Running', className: 'text-info' },
}

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
    const [searchQuery, setSearchQuery] = useState('')
    const [expandedParents, setExpandedParents] = useState<Set<string>>(() => new Set())
    // eslint-disable-next-line react-hooks/purity -- render-time clock for run ages; the list re-renders on run updates
    const now = Date.now()
    const query = searchQuery.trim().toLowerCase()
    const matchesSearch = (run: RunRecord) => (
        !query
        || runTitle(run).toLowerCase().includes(query)
        || flowTitle(run.flow_name).toLowerCase().includes(query)
        || run.flow_name.toLowerCase().includes(query)
    )
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
    const visibleQueuedLockGroups = queuedLockGroups
        .map((group) => ({ ...group, runs: group.runs.filter(matchesSearch) }))
        .filter((group) => group.runs.length > 0)

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
        } else if (matchesSearch(run)) {
            topLevelRuns.push(run)
        }
    }
    const needsInputRuns = topLevelRuns.filter((run) => run.status === 'waiting')
    const runningRuns = topLevelRuns.filter((run) => ACTIVE_LIST_STATUSES.has(run.status))
    const recentRuns = topLevelRuns.filter(
        (run) => run.status !== 'waiting' && !ACTIVE_LIST_STATUSES.has(run.status),
    )

    const renderRunRow = (run: RunRecord, depth = 0) => {
        const projectLabel = scopeMode === 'all' ? compactProjectLabel(run.project_path) : null
        const attention = ATTENTION[run.status]
        const title = depth === 0 ? runTitle(run) : flowTitle(run.flow_name)
        const metaParts = [
            title === flowTitle(run.flow_name) ? null : flowTitle(run.flow_name),
            formatRunAge(run.started_at, now),
            formatDuration(run.started_at, run.ended_at, run.status, now),
            projectLabel,
        ].filter((value): value is string => Boolean(value) && value !== '—')
        const holdsExecutionLock = run.execution_lock?.state === 'holding'
        const queuedForExecutionLock = run.execution_lock?.state === 'queued'
        const childRuns = childRunsByParent.get(run.run_id) ?? []
        const childrenExpanded = expandedParents.has(run.run_id)

        return (
            <div key={run.run_id} className={cn(depth > 0 && 'ml-4 border-l border-border/60 pl-2')}>
            <article
                data-testid="run-history-row"
                data-run-id={run.run_id}
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
                    'rounded-none px-3 py-1.5 outline-none transition-colors hover:text-primary focus-visible:ring-2 focus-visible:ring-primary/30 cursor-pointer',
                    selectedRunId === run.run_id && 'text-primary shadow-[inset_2px_0_0_hsl(var(--primary))]',
                )}
            >
                <div data-testid="run-history-row-title" className="truncate text-sm font-normal" title={`${title} · ${run.run_id}`}>
                    {title}
                </div>
                <div data-testid="run-history-row-meta" className="truncate text-xs leading-4 text-muted-foreground">
                    {metaParts.join(' · ')}
                    {attention ? (
                        <span data-testid="run-history-row-status" className={attention.className}>
                            {metaParts.length > 0 ? ' · ' : ''}{attention.label}
                        </span>
                    ) : null}
                </div>
                {holdsExecutionLock ? (
                    <div className="text-xs font-medium text-warning">
                        Holding execution lock
                    </div>
                ) : null}
                {queuedForExecutionLock ? (
                    <div className="text-xs font-medium text-warning">
                        Queued for execution lock{typeof run.execution_lock?.queue_position === 'number'
                            ? ` · position ${run.execution_lock.queue_position}`
                            : ''}
                    </div>
                ) : null}
            </article>
            {childRuns.length > 0 ? (
                <div className="ml-3">
                    <button
                        type="button"
                        data-testid="run-history-children-toggle"
                        aria-expanded={childrenExpanded}
                        onClick={() => setExpandedParents((current) => {
                            const next = new Set(current)
                            if (!next.delete(run.run_id)) {
                                next.add(run.run_id)
                            }
                            return next
                        })}
                        className="px-3 text-xs text-muted-foreground hover:text-foreground"
                    >
                        {childrenExpanded ? '▾' : '▸'} {childRuns.length} child {childRuns.length === 1 ? 'run' : 'runs'}
                    </button>
                    {childrenExpanded ? (
                        <div data-testid="run-history-children" className="space-y-1">
                            {childRuns.map((child) => renderRunRow(child, depth + 1))}
                        </div>
                    ) : null}
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
                    'flex items-center justify-between px-1 text-xs font-medium uppercase tracking-wide text-muted-foreground',
                    accent,
                )}
                >
                    <span>{label}</span>
                    <span data-testid={`run-list-group-${key}-count`}>{groupRuns.length}</span>
                </div>
                <div className="space-y-2">
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
                isNarrowViewport ? 'w-full max-h-[46vh] rounded-md border' : 'w-64 border-r'
            }`}
        >
            <div className="space-y-2 px-3 pb-2 pt-3">
                <div className="flex items-center justify-between gap-2">
                    <Badge
                        data-testid="runs-project-context-chip"
                        variant="outline"
                        className="min-w-0"
                        title={activeProjectPath || 'No active project'}
                    >
                        <span className="text-muted-foreground">Project:</span>
                        <span className="max-w-28 truncate">{activeProjectLabel}</span>
                    </Badge>
                    <span
                        data-testid="runs-scope-description"
                        className="min-w-0 truncate text-xs text-muted-foreground"
                        title={scopeDescription}
                    >
                        {summaryLabel}
                    </span>
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
                <Input
                    type="search"
                    value={searchQuery}
                    onChange={(event) => setSearchQuery(event.target.value)}
                    placeholder="Search runs…"
                    aria-label="Search runs by title or flow"
                    data-testid="run-list-search-input"
                    className="h-7 text-sm"
                />
                {error ? (
                    <InlineError>{error}</InlineError>
                ) : null}
                {scopeMode === 'active' && !activeProjectPath ? (
                    <Alert className="border-border px-3 py-2 text-muted-foreground">
                        <AlertDescription className="text-inherit">
                            Choose an active project or switch to all projects to view run history.
                        </AlertDescription>
                    </Alert>
                ) : null}
            </div>
            {status !== 'ready' && status !== 'error' && runs.length === 0 ? (
                <div className="px-4 pb-4">
                    <p data-testid="run-list-loading" className="text-sm text-muted-foreground" aria-live="polite">Restoring run history…</p>
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
                    <div className="space-y-3">
                        {renderRunGroup('needs-input', 'Needs input', needsInputRuns, 'text-warning')}
                        {renderRunGroup('running', 'Running', runningRuns)}
                        {visibleQueuedLockGroups.map((group) => (
                            <section key={group.identity} className="space-y-2">
                                <div className="rounded-md border-0 border-l border-warning px-3 py-2 text-xs font-medium text-warning">
                                    Queued execution lock · {group.label}
                                </div>
                                <div className="space-y-3">
                                    {group.runs.map((run) => renderRunRow(run))}
                                </div>
                            </section>
                        ))}
                        {renderRunGroup('recent', 'Recent', recentRuns)}
                        {query && topLevelRuns.length === 0 && visibleQueuedLockGroups.length === 0 ? (
                            <p data-testid="run-list-search-empty" className="px-1 text-xs text-muted-foreground">
                                No runs match the search.
                            </p>
                        ) : null}
                    </div>
                </div>
            )}
        </nav>
    )
}
