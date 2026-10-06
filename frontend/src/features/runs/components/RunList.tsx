import { useState } from 'react'
import { ChevronDown, Layers, Search } from 'lucide-react'

import { cn } from '@/lib/utils'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { InlineError } from '@/components/app/inline-error'
import { ListGroupLabel, SidePanelRow } from '@/components/app/side-panel-row'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { useStore } from '@/store'
import { projectLabel } from '@/features/projects/model/projectChoices'
import type { RunRecord } from '../model/shared'
import { formatDuration, runStatusTone } from '../model/shared'
import { flowTitle, formatRunDate, runDayLabel, runTitle } from '../model/runOverviewModel'

const ACTIVE_LIST_STATUSES = new Set([
    'running',
    'queued',
    'pause_requested',
    'abort_requested',
    'cancel_requested',
])

// A status word only when the run needs attention, and never one the group label
// already says ("Running" under Running).
const ATTENTION: Record<string, { label: string; className: string }> = {
    failed: { label: 'Failed', className: 'text-destructive' },
    validation_error: { label: 'Failed', className: 'text-destructive' },
    waiting: { label: 'Needs input', className: 'text-warning' },
    running: { label: 'Running', className: 'text-info' },
    queued: { label: 'Queued', className: 'text-muted-foreground' },
}

interface RunListProps {
    error: string | null
    status: 'idle' | 'loading' | 'ready' | 'error'
    onSelectRun: (run: RunRecord) => void
    runs: RunRecord[]
    selectedRunId: string | null
    /** Fills the search placeholder: "Search 233 runs". */
    summaryLabel: string
    /** Name each row's project; off when the list is already filtered to one project. */
    showProject?: boolean
}

export function RunList({
    error,
    status,
    onSelectRun,
    runs,
    selectedRunId,
    summaryLabel,
    showProject = true,
}: RunListProps) {
    const isNarrowViewport = useNarrowViewport()
    const projectRegistry = useStore((state) => state.projectRegistry)
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
    // orphaned) runs bucket by how actionable they are, and finished runs by day.
    const listedRunIds = new Set(historyRuns.map((run) => run.run_id))
    const childRunsByParent = new Map<string, RunRecord[]>()
    const rootRuns: RunRecord[] = []
    for (const run of historyRuns) {
        if (run.parent_run_id && listedRunIds.has(run.parent_run_id)) {
            const siblings = childRunsByParent.get(run.parent_run_id) ?? []
            siblings.push(run)
            childRunsByParent.set(run.parent_run_id, siblings)
        } else {
            rootRuns.push(run)
        }
    }
    // A search keeps a run whose own row or any descendant matches, so a
    // matching child stays reachable through its ancestors.
    const descendantMatches = (run: RunRecord, seen = new Set<string>()): boolean => {
        seen.add(run.run_id)
        return (childRunsByParent.get(run.run_id) ?? []).some(
            (child) => !seen.has(child.run_id) && (matchesSearch(child) || descendantMatches(child, seen)),
        )
    }
    const topLevelRuns = rootRuns.filter((run) => matchesSearch(run) || descendantMatches(run))
    const needsInputRuns = topLevelRuns.filter((run) => run.status === 'waiting')
    const runningRuns = topLevelRuns.filter((run) => ACTIVE_LIST_STATUSES.has(run.status))
    const recentRuns = topLevelRuns.filter(
        (run) => run.status !== 'waiting' && !ACTIVE_LIST_STATUSES.has(run.status),
    )
    const dayGroups = new Map<string, RunRecord[]>()
    for (const run of recentRuns) {
        const label = runDayLabel(run.started_at, now) || 'Undated'
        dayGroups.set(label, [...(dayGroups.get(label) ?? []), run])
    }

    // The row drops what its group label already states: the date under a day group
    // (`underDay`) and the status word under a state group (`stated`). Child rows also
    // drop the project their parent names.
    const renderRunRow = (
        run: RunRecord,
        { depth = 0, underDay = false, stated }: { depth?: number; underDay?: boolean; stated?: string } = {},
    ) => {
        const runProject = depth === 0 && showProject && run.project_path ? projectLabel(projectRegistry, run.project_path) : null
        const attention = ATTENTION[run.status]?.label === stated ? null : ATTENTION[run.status]
        const title = depth === 0 ? runTitle(run) : flowTitle(run.flow_name)
        const metaParts = [
            title === flowTitle(run.flow_name) ? null : flowTitle(run.flow_name),
            underDay ? null : formatRunDate(run.started_at, now),
            formatDuration(run.started_at, run.ended_at, run.status, now),
        ].filter((value): value is string => Boolean(value) && value !== '—')
        // Short enough for the trailing slot, which never truncates (the meta line does).
        const lock = run.execution_lock?.state === 'holding'
            ? 'Holding lock'
            : run.execution_lock?.state === 'queued'
                ? `Lock queue${typeof run.execution_lock.queue_position === 'number' ? ` #${run.execution_lock.queue_position}` : ''}`
                : null
        const allChildRuns = childRunsByParent.get(run.run_id) ?? []
        // Searching opens the path to matching descendants; a matching parent
        // keeps all of its children available.
        const revealsMatch = Boolean(query) && descendantMatches(run)
        const childRuns = query && !matchesSearch(run)
            ? allChildRuns.filter((child) => matchesSearch(child) || descendantMatches(child))
            : allChildRuns
        const childrenExpanded = revealsMatch || expandedParents.has(run.run_id)
        const selected = selectedRunId === run.run_id

        return (
            <div key={run.run_id} className={cn(depth > 0 && 'ml-4')}>
                <div className="relative">
                    {/* The trailing slot reserves the chip's corner; the children toggle overlays it
                        below, so a button never nests inside the row button. */}
                    <SidePanelRow
                        data-testid="run-history-row"
                        data-run-id={run.run_id}
                        selected={selected}
                        tone={runStatusTone(run.status)}
                        onClick={() => onSelectRun(run)}
                        title={<span data-testid="run-history-row-title" title={`${title} · ${run.run_id}`}>{title}</span>}
                        meta={(
                            <span data-testid="run-history-row-meta">
                                {runProject ? (
                                    <span data-testid="run-history-row-project" title={run.project_path ?? undefined}>
                                        {runProject}{metaParts.length > 0 ? ' · ' : ''}
                                    </span>
                                ) : null}
                                {metaParts.join(' · ')}
                                {attention ? (
                                    <span data-testid="run-history-row-status" className={attention.className}>
                                        {metaParts.length > 0 ? ' · ' : ''}{attention.label}
                                    </span>
                                ) : null}
                            </span>
                        )}
                        trailing={lock || childRuns.length > 0 ? (
                            <>
                                {lock ? <span data-testid="run-history-row-lock" className="font-medium text-warning">{lock}</span> : null}
                                {childRuns.length > 0 ? (
                                    <span aria-hidden="true" className="invisible flex items-center gap-1 tabular-nums"><Layers className="size-3.5" />{childRuns.length}</span>
                                ) : null}
                            </>
                        ) : null}
                    />
                    {childRuns.length > 0 ? (
                        <button
                            type="button"
                            data-testid="run-history-children-toggle"
                            aria-expanded={childrenExpanded}
                            aria-label={`${childrenExpanded ? 'Hide' : 'Show'} ${childRuns.length} child ${childRuns.length === 1 ? 'run' : 'runs'}`}
                            title={`${childRuns.length} child ${childRuns.length === 1 ? 'run' : 'runs'}`}
                            onClick={() => {
                                setExpandedParents((current) => {
                                    const next = new Set(current)
                                    if (!next.delete(run.run_id)) {
                                        next.add(run.run_id)
                                    }
                                    return next
                                })
                            }}
                            className="absolute right-2.5 top-2.5 flex items-center gap-1 rounded-sm text-xs tabular-nums text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                        >
                            {childrenExpanded ? <ChevronDown className="size-3.5" /> : <Layers className="size-3.5" />}
                            {childRuns.length}
                        </button>
                    ) : null}
                </div>
                {childRuns.length > 0 && childrenExpanded ? (
                    <div data-testid="run-history-children" className="ml-3 space-y-0.5">
                        {childRuns.map((child) => renderRunRow(child, { depth: depth + 1, underDay, stated }))}
                    </div>
                ) : null}
            </div>
        )
    }

    const renderRunGroup = (key: string, label: string, groupRuns: RunRecord[], labelClassName?: string) => {
        if (groupRuns.length === 0) {
            return null
        }
        return (
            <section key={key} data-testid={`run-list-group-${key}`} className="space-y-0.5">
                <ListGroupLabel className={labelClassName}>
                    {label} · <span data-testid={`run-list-group-${key}-count`}>{groupRuns.length}</span>
                </ListGroupLabel>
                {groupRuns.map((run) => renderRunRow(run, { stated: label }))}
            </section>
        )
    }

    return (
        <nav
            data-testid="run-list-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className="flex h-full min-h-0 w-full flex-col overflow-hidden"
        >
            <div className="space-y-2 pb-3">
                <label className="flex h-8 items-center gap-2 rounded-lg border border-border bg-background px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
                    <Search aria-hidden="true" className="size-3.5 shrink-0" />
                    <input
                        type="search"
                        value={searchQuery}
                        onChange={(event) => setSearchQuery(event.target.value)}
                        placeholder={`Search ${summaryLabel}`}
                        aria-label="Search runs by title or flow"
                        data-testid="run-list-search-input"
                        className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                    />
                </label>
                {error ? (
                    <InlineError>{error}</InlineError>
                ) : null}
            </div>
            {status !== 'ready' && status !== 'error' && runs.length === 0 ? (
                <p data-testid="run-list-loading" className="px-3 text-sm text-muted-foreground" aria-live="polite">Restoring run history…</p>
            ) : runs.length === 0 ? (
                <Empty className="py-4 text-xs">
                    <EmptyHeader>
                        <EmptyDescription>No runs yet.</EmptyDescription>
                    </EmptyHeader>
                </Empty>
            ) : (
                <div
                    data-testid="run-list-scroll-region"
                    className="min-h-0 flex-1 space-y-3 overflow-y-auto pb-4"
                >
                    {renderRunGroup('needs-input', 'Needs input', needsInputRuns, 'text-warning')}
                    {renderRunGroup('running', 'Running', runningRuns)}
                    {visibleQueuedLockGroups.map((group) => (
                        <section key={group.identity} className="space-y-0.5">
                            <ListGroupLabel className="text-warning">Queued execution lock · {group.label}</ListGroupLabel>
                            {group.runs.map((run) => renderRunRow(run, { stated: 'Queued' }))}
                        </section>
                    ))}
                    {recentRuns.length > 0 ? (
                        <section data-testid="run-list-group-recent" className="space-y-3">
                            {[...dayGroups].map(([label, groupRuns]) => (
                                <div key={label} data-testid="run-list-day-group" className="space-y-0.5">
                                    <ListGroupLabel>{label}</ListGroupLabel>
                                    {groupRuns.map((run) => renderRunRow(run, { underDay: true }))}
                                </div>
                            ))}
                        </section>
                    ) : null}
                    {query && topLevelRuns.length === 0 && visibleQueuedLockGroups.length === 0 ? (
                        <p data-testid="run-list-search-empty" className="px-3 text-xs text-muted-foreground">
                            No runs match the search.
                        </p>
                    ) : null}
                </div>
            )}
        </nav>
    )
}
