import { useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import { ChevronDown, ChevronRight, Layers, Search } from 'lucide-react'

import { cn } from '@/lib/utils'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useNow } from '@/lib/useNow'
import { InlineError } from '@/components/app/inline-error'
import { ListGroupLabel, SidePanelRow } from '@/components/app/side-panel-row'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { useStore } from '@/store'
import { UNREACHABLE_MESSAGE } from '@/lib/fetchSpark'
import { projectLabel } from '@/features/projects/model/projectChoices'
import type { RunRecord } from '../model/shared'
import { formatDuration, formatRunStatusLabel, formatTimestamp, isCompletedFailure, runHasEnded, runRecordTone } from '../model/shared'
import { requestRunsTransportReconnect } from '../services/runsTransportReconnect'
import { flowTitle, formatRunDate, formatRunTime, runDayLabel, runKind, runMatchesSearch, runTitle, type RunStatusKind } from '../model/runOverviewModel'

const ACTIVE_LIST_STATUSES = new Set([
    'running',
    'queued',
    'pause_requested',
    'abort_requested',
    'cancel_requested',
])

// A status word when the run needs attention, or when its dot alone wouldn't say how it ended
// (a canceled run's grey), and never one the group label already says ("Running" under Running).
const ATTENTION: Record<string, { label: string; className: string }> = {
    canceled: { label: 'Canceled', className: 'text-muted-foreground' },
    aborted: { label: 'Canceled', className: 'text-muted-foreground' },
    failed: { label: 'Failed', className: 'text-destructive' },
    validation_error: { label: 'Failed', className: 'text-destructive' },
    unreadable: { label: 'Unreadable', className: 'text-destructive' },
    waiting: { label: 'Needs input', className: 'text-warning' },
    running: { label: 'Running', className: 'text-info' },
    queued: { label: 'Queued', className: 'text-muted-foreground' },
}
// A run that completed at a failure exit says which exit ("Changes requested"), as its header does.
const attentionOf = (run: RunRecord) => (isCompletedFailure(run)
    ? { label: formatRunStatusLabel(run), className: 'text-destructive' }
    : ATTENTION[run.status])

interface RunListProps {
    error: string | null
    status: 'idle' | 'loading' | 'ready' | 'error'
    onSelectRun: (run: RunRecord) => void
    runs: RunRecord[]
    selectedRunId: string | null
    /** Fills the search placeholder: "Search 233 runs". */
    summaryLabel: string
    /** The search as typed; held by the caller, so opening a run it would hide can clear it. */
    searchQuery?: string
    onSearchQueryChange?: (searchQuery: string) => void
    /** Name each row's project; off when the list is already filtered to one project. */
    showProject?: boolean
    /** Show only runs in this state; null shows all. */
    statusFilter?: RunStatusKind | null
    onStatusFilterChange?: (statusFilter: RunStatusKind | null) => void
}

// The states a run list can be narrowed to, in the order the list groups them.
const STATUS_FILTERS: Array<{ kind: RunStatusKind; label: string; className?: string }> = [
    { kind: 'waiting', label: 'Needs input', className: 'text-warning' },
    { kind: 'running', label: 'Running' },
    { kind: 'failed', label: 'Failed', className: 'text-destructive' },
    { kind: 'completed', label: 'Completed' },
    { kind: 'ended', label: 'Canceled' },
]
// A day's repeats of one run (same title, flow, project and outcome) fold into one row from this many on.
const REPEATS_FOLDED_FROM = 3
// An open fold lists this many of its runs at a time.
const REPEATS_PAGE = 10

export function RunList({
    error,
    status,
    onSelectRun,
    runs,
    selectedRunId,
    summaryLabel,
    searchQuery: heldSearchQuery,
    onSearchQueryChange,
    showProject = true,
    statusFilter = null,
    onStatusFilterChange,
}: RunListProps) {
    const isNarrowViewport = useNarrowViewport()
    const projectRegistry = useStore((state) => state.projectRegistry)
    // The list is one Tab stop, as the steps list is: the focused row while focus is in the list,
    // else the open run's, else the first. Arrows, j/k, Home and End move between its rows and
    // toggles; Enter or Space opens one, since opening a run loads it and adds a history entry.
    const listRef = useRef<HTMLDivElement>(null)
    const keysHintId = useId()
    const placeTabStop = (focused: EventTarget | null = document.activeElement) => {
        const items = [...listRef.current?.querySelectorAll('button') ?? []]
        const stop = items.find((item) => item === focused)
            ?? items.find((item) => item.getAttribute('aria-current') === 'true')
            ?? items[0]
        items.forEach((item) => { item.tabIndex = item === stop ? 0 : -1 })
    }
    useLayoutEffect(() => placeTabStop())
    const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.metaKey || event.ctrlKey || event.altKey) return
        const items = [...event.currentTarget.querySelectorAll('button')]
        const index = items.indexOf(event.target as HTMLButtonElement)
        const moves: Record<string, number> = { ArrowDown: index + 1, j: index + 1, ArrowUp: index - 1, k: index - 1, Home: 0, End: items.length - 1 }
        if (index < 0 || !Object.hasOwn(moves, event.key)) return
        event.preventDefault()
        items[moves[event.key]]?.focus()
    }
    const [ownSearchQuery, setOwnSearchQuery] = useState('')
    const searchQuery = heldSearchQuery ?? ownSearchQuery
    const setSearchQuery = onSearchQueryChange ?? setOwnSearchQuery
    const [expandedParents, setExpandedParents] = useState<Set<string>>(() => new Set())
    // How many runs each open fold lists, by fold key; REPEATS_PAGE when absent.
    const [foldShown, setFoldShown] = useState<Record<string, number>>({})
    const now = useNow(runs.some((run) => !run.ended_at && !runHasEnded(run.status) && run.status !== 'queued'))
    const query = searchQuery.trim().toLowerCase()
    const statusCounts = runs.reduce<Partial<Record<RunStatusKind, number>>>((counts, run) => {
        const kind = runKind(run)
        counts[kind] = (counts[kind] ?? 0) + 1
        return counts
    }, {})
    // A search matches a run's title, its flow, or its state ("failed", "needs input").
    const matchesSearch = (run: RunRecord) => (!statusFilter || runKind(run) === statusFilter) && runMatchesSearch(run, query)
    const filtering = Boolean(query) || Boolean(statusFilter)
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
    const runsById = new Map(runs.map((run) => [run.run_id, run]))
    // A run opened from elsewhere (the Overview, the Inbox) is listed: its parents open.
    // (Opening it also clears a search that would hide it; see setRunsSelectedRunId.)
    const selectedAncestors = new Set<string>()
    let ancestor = selectedRunId ? runsById.get(selectedRunId)?.parent_run_id : undefined
    while (ancestor && runsById.has(ancestor) && !selectedAncestors.has(ancestor)) {
        selectedAncestors.add(ancestor)
        ancestor = runsById.get(ancestor)?.parent_run_id
    }
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

    // The row drops what its group label already states: the day under a day group
    // (`underDay`, which keeps the time) and the status word under a state group (`stated`).
    // Child rows also drop the project their parent names.
    const renderRunRow = (
        run: RunRecord,
        { depth = 0, underDay = false, stated }: { depth?: number; underDay?: boolean; stated?: string } = {},
    ) => {
        const runProject = depth === 0 && showProject && run.project_path ? projectLabel(projectRegistry, run.project_path) : null
        const attention = attentionOf(run)?.label === stated ? null : attentionOf(run)
        const title = depth === 0 ? runTitle(run) : flowTitle(run.flow_name)
        const flowPart = title === flowTitle(run.flow_name) ? null : flowTitle(run.flow_name)
        const metaParts = [
            underDay ? formatRunTime(run.started_at) : formatRunDate(run.started_at, now),
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
        const revealsMatch = filtering && descendantMatches(run)
        const childRuns = filtering && !matchesSearch(run)
            ? allChildRuns.filter((child) => matchesSearch(child) || descendantMatches(child))
            : allChildRuns
        // Open by default when it holds the open run; the toggle flips the default.
        const childrenExpanded = revealsMatch || selectedAncestors.has(run.run_id) !== expandedParents.has(run.run_id)
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
                        tone={runRecordTone(run)}
                        onClick={() => onSelectRun(run)}
                        title={<span data-testid="run-history-row-title" title={`${title} · ${run.run_id}`}>{title}</span>}
                        meta={(
                            // The time, duration and status word lead and always show; the project
                            // and flow follow and give way at the line's end when it is short.
                            <span data-testid="run-history-row-meta" className="flex min-w-0">
                                <span className="shrink-0 whitespace-pre" title={run.started_at ? `Started ${formatTimestamp(run.started_at)}` : undefined}>
                                    {metaParts.join(' · ')}
                                    {attention ? (
                                        <span data-testid="run-history-row-status" className={attention.className}>
                                            {metaParts.length > 0 ? ' · ' : ''}{attention.label}
                                        </span>
                                    ) : null}
                                </span>
                                {runProject || flowPart ? (
                                    <span className="min-w-0 truncate whitespace-pre">
                                        {metaParts.length > 0 || attention ? ' · ' : ''}
                                        {runProject ? (
                                            <span data-testid="run-history-row-project" title={run.project_path ?? undefined}>{runProject}</span>
                                        ) : null}
                                        {runProject && flowPart ? ' · ' : ''}{flowPart}
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
                            onClick={() => toggleExpanded(run.run_id)}
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

    const toggleExpanded = (key: string) => setExpandedParents((current) => {
        const next = new Set(current)
        if (!next.delete(key)) next.add(key)
        return next
    })

    // A day's repeats fold into one row at the latest one's place; the rest keep their order.
    const renderDayRuns = (day: string, dayRuns: RunRecord[]) => {
        const keyOf = (run: RunRecord) => (childRunsByParent.has(run.run_id)
            ? run.run_id
            : [runTitle(run), run.flow_name, run.project_path, runKind(run), attentionOf(run)?.label].join('\n'))
        const repeats = new Map<string, RunRecord[]>()
        for (const run of dayRuns) repeats.set(keyOf(run), [...(repeats.get(keyOf(run)) ?? []), run])
        return dayRuns.map((run) => {
            const same = repeats.get(keyOf(run)) ?? [run]
            if (same.length < REPEATS_FOLDED_FROM) return renderRunRow(run, { underDay: true })
            return same[0] === run ? renderRepeats(`${day}\n${keyOf(run)}`, same) : null
        })
    }

    // A fold's runs differ only in when they ran, so each lists as its start time, to the second.
    const renderRepeats = (key: string, repeatRuns: RunRecord[]) => {
        const latest = repeatRuns[0]
        const selectedIndex = repeatRuns.findIndex((run) => run.run_id === selectedRunId)
        const open = (selectedIndex >= 0) !== expandedParents.has(key)
        const attention = attentionOf(latest)
        const runProject = showProject && latest.project_path ? projectLabel(projectRegistry, latest.project_path) : null
        const flowPart = runTitle(latest) === flowTitle(latest.flow_name) ? null : flowTitle(latest.flow_name)
        const shown = Math.max(foldShown[key] ?? REPEATS_PAGE, selectedIndex + 1)
        const more = repeatRuns.length - shown
        return (
            <div key={key} className="space-y-0.5">
                <SidePanelRow
                    data-testid="run-history-repeats"
                    aria-expanded={open}
                    tone={runRecordTone(latest)}
                    onClick={() => toggleExpanded(key)}
                    title={runTitle(latest)}
                    meta={(
                        // The count and status word lead and always show; the project and flow give way.
                        <span className="flex min-w-0">
                            <span className="shrink-0 whitespace-pre" title={`Latest started ${formatTimestamp(latest.started_at)}`}>
                                {repeatRuns.length} runs
                                {attention ? <span className={attention.className}> · {attention.label}</span> : null}
                                {' · '}{formatRunTime(latest.started_at)}
                            </span>
                            {runProject || flowPart ? (
                                <span className="min-w-0 truncate whitespace-pre"> · {[runProject, flowPart].filter(Boolean).join(' · ')}</span>
                            ) : null}
                        </span>
                    )}
                    trailing={open ? <ChevronDown aria-hidden="true" className="size-3.5" /> : <ChevronRight aria-hidden="true" className="size-3.5" />}
                />
                {open ? (
                    <div data-testid="run-history-repeats-runs" className="ml-4 space-y-0.5">
                        {repeatRuns.slice(0, shown).map((run) => (
                            <SidePanelRow
                                key={run.run_id}
                                data-testid="run-history-row"
                                data-run-id={run.run_id}
                                selected={selectedRunId === run.run_id}
                                tone={runRecordTone(run)}
                                onClick={() => onSelectRun(run)}
                                title={<span data-testid="run-history-row-title" title={`${runTitle(run)} · ${run.run_id}`}>{formatRunTime(run.started_at, true) || runTitle(run)}</span>}
                                meta={<span data-testid="run-history-row-meta">{formatDuration(run.started_at, run.ended_at, run.status, now)}</span>}
                            />
                        ))}
                        {more > 0 ? (
                            <button
                                type="button"
                                data-testid="run-history-repeats-more"
                                onClick={() => setFoldShown((current) => ({ ...current, [key]: shown + REPEATS_PAGE * 5 }))}
                                className="px-3 py-1 text-xs text-muted-foreground underline underline-offset-4 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                            >
                                Show {Math.min(more, REPEATS_PAGE * 5)} more of {repeatRuns.length}
                            </button>
                        ) : null}
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
            aria-label="Runs"
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
                        aria-label="Search runs by title, flow or state"
                        data-testid="run-list-search-input"
                        className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                    />
                </label>
                {onStatusFilterChange && runs.length > 0 ? (
                    <div role="group" aria-label="Show runs that are" data-testid="run-list-status-filter" className="flex flex-wrap gap-1">
                        {[{ kind: null, label: 'All', className: undefined }, ...STATUS_FILTERS].map(({ kind, label, className }) => {
                            const count = kind ? statusCounts[kind] ?? 0 : runs.length
                            // A state with no runs has nothing to show, unless it is the one chosen.
                            if (kind && count === 0 && statusFilter !== kind) return null
                            const pressed = statusFilter === kind
                            return (
                                <button
                                    key={kind ?? 'all'}
                                    type="button"
                                    aria-pressed={pressed}
                                    data-testid={`run-list-status-filter-${kind ?? 'all'}`}
                                    onClick={() => onStatusFilterChange(kind)}
                                    className={cn(
                                        'flex h-6 items-center gap-1 rounded-full border px-2 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
                                        pressed ? 'border-foreground/30 bg-muted font-medium text-foreground' : 'border-border text-muted-foreground hover:text-foreground',
                                    )}
                                >
                                    <span className={kind && count > 0 ? className : undefined}>{label}</span>
                                    <span className="tabular-nums text-muted-foreground">{count}</span>
                                </button>
                            )
                        })}
                    </div>
                ) : null}
                {/* With no runs to show, the view's main pane says the load failed. */}
                {error && runs.length > 0 ? (
                    <InlineError dense data-testid="run-list-error">
                        {error === UNREACHABLE_MESSAGE ? 'Couldn’t reach Spark; showing the last known runs.' : 'Couldn’t refresh your runs; this list may be out of date.'}
                        <button type="button" onClick={requestRunsTransportReconnect} className="ml-1 font-medium underline underline-offset-4">Try again</button>
                    </InlineError>
                ) : null}
            </div>
            {runs.length === 0 ? (
                status === 'ready' ? (
                    <Empty className="py-4 text-xs">
                        <EmptyHeader>
                            <EmptyDescription>No runs yet.</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                ) : status === 'error' ? null : (
                    <p data-testid="run-list-loading" className="px-3 text-sm text-muted-foreground" aria-live="polite">Loading runs…</p>
                )
            ) : (
                <div
                    ref={listRef}
                    role="group"
                    aria-label="Run list"
                    aria-describedby={keysHintId}
                    data-testid="run-list-scroll-region"
                    data-own-keys="jk"
                    onKeyDown={onListKeyDown}
                    onFocus={(event) => placeTabStop(event.target)}
                    onBlur={(event) => placeTabStop(event.relatedTarget)}
                    className="min-h-0 flex-1 space-y-3 overflow-y-auto pb-4"
                >
                    <p id={keysHintId} className="sr-only">Up and down arrows, or J and K, move between runs; Enter opens one.</p>
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
                                    {renderDayRuns(label, groupRuns)}
                                </div>
                            ))}
                        </section>
                    ) : null}
                    {filtering && topLevelRuns.length === 0 && visibleQueuedLockGroups.length === 0 ? (
                        <p data-testid="run-list-search-empty" className="px-3 text-xs text-muted-foreground">
                            {query ? 'No runs match the search.' : 'No runs in this state.'}
                            {statusFilter && onStatusFilterChange ? (
                                <button type="button" onClick={() => onStatusFilterChange(null)} className="ml-1 underline underline-offset-4 hover:text-foreground">Show all runs</button>
                            ) : null}
                        </p>
                    ) : null}
                </div>
            )}
        </nav>
    )
}
