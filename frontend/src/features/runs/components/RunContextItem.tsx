import { useEffect, useRef, useState } from 'react'

import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import type { ContextErrorState } from '../model/shared'
import type { ContextKeyHistory, RunContextOverview } from '../model/runOverviewModel'
import type { RunVisit } from '../model/visitModel'
import { VisitLink } from './RunStatusItem'

const formatValue = (value: unknown): string => (
    typeof value === 'string' ? value : JSON.stringify(value)
)

// Expanded values keep their line breaks and indent structured values.
const formatFullValue = (value: unknown): string => (
    typeof value === 'string' ? value : JSON.stringify(value, null, 2)
)

// ponytail: length heuristic; a compact row may still clip a shorter value on narrow screens.
const isLongValue = (value: unknown): boolean => (
    value !== null && (typeof value === 'object' || formatValue(value).length > 60 || formatValue(value).includes('\n'))
)

const matches = (row: ContextKeyHistory, query: string): boolean => (
    !query
    || row.key.toLowerCase().includes(query)
    || (row.value !== null && formatValue(row.value).toLowerCase().includes(query))
)

interface RunContextItemProps {
    overview: RunContextOverview
    finalContext: Record<string, unknown> | null
    status: 'idle' | 'loading' | 'ready' | 'error'
    contextError: ContextErrorState | null
    searchQuery: string
    onSearchQueryChange: (query: string) => void
    contextCopyStatus: string
    contextExportHref: string | null
    onCopy: () => void
    onRefresh: () => void
    /** A key opened from a visit's reads or writes. */
    focusKey: string | null
    onSelectVisit: (visit: RunVisit) => void
}

export function RunContextItem({
    overview,
    finalContext,
    status,
    contextError,
    searchQuery,
    onSearchQueryChange,
    contextCopyStatus,
    contextExportHref,
    onCopy,
    onRefresh,
    focusKey,
    onSelectVisit,
}: RunContextItemProps) {
    const [expandedKeys, setExpandedKeys] = useState<Set<string>>(() => new Set(focusKey ? [focusKey] : []))
    const rootRef = useRef<HTMLElement | null>(null)
    // Opening a key reveals it once, clearing a search that hides it first.
    const pendingFocusRef = useRef(focusKey)
    const query = searchQuery.trim().toLowerCase()
    const focusRow = overview.namespaces.flatMap((namespace) => namespace.keys).find((row) => row.key === focusKey)
    const focusHidden = focusRow !== undefined && !matches(focusRow, query)
    useEffect(() => {
        const key = pendingFocusRef.current
        if (!key || !focusRow) return
        if (focusHidden) {
            onSearchQueryChange('')
            return
        }
        pendingFocusRef.current = null
        rootRef.current?.querySelector(`[data-context-key="${CSS.escape(key)}"]`)?.scrollIntoView({ block: 'center' })
    }, [focusHidden, focusRow, onSearchQueryChange])
    const namespaces = overview.namespaces
        .map((namespace) => ({ ...namespace, keys: namespace.keys.filter((row) => matches(row, query)) }))
        .filter((namespace) => namespace.keys.length > 0)
    const systemKeys = overview.systemKeys.filter((key) => !query || key.toLowerCase().includes(query))
    const toggle = (key: string) => setExpandedKeys((current) => {
        const next = new Set(current)
        if (!next.delete(key)) {
            next.add(key)
        }
        return next
    })
    const actionClass = 'text-xs text-muted-foreground hover:text-foreground'

    return (
        <article ref={rootRef} data-testid="run-context-panel" className="space-y-3 pb-6">
            <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h3 className="text-lg font-light text-foreground">Context</h3>
                <span className="text-sm text-muted-foreground">Final values · the visit that last wrote each</span>
            </header>
            <div className="flex flex-wrap items-center gap-3">
                <Input
                    value={searchQuery}
                    onChange={(event) => onSearchQueryChange(event.target.value)}
                    placeholder="Search keys and values…"
                    data-testid="run-context-search-input"
                    className="h-7 max-w-xs flex-1 text-sm"
                />
                <button type="button" data-testid="run-context-refresh-button" onClick={onRefresh} className={actionClass}>
                    {status === 'loading' ? 'Refreshing…' : 'Refresh'}
                </button>
                <button type="button" data-testid="run-context-copy-button" onClick={onCopy} className={actionClass}>
                    Copy JSON
                </button>
                {contextExportHref ? (
                    <a data-testid="run-context-export-button" href={contextExportHref} download="run-context.json" className={actionClass}>
                        Export JSON
                    </a>
                ) : null}
                {contextCopyStatus ? (
                    <span data-testid="run-context-copy-status" className="text-xs text-muted-foreground">{contextCopyStatus}</span>
                ) : null}
            </div>
            {contextError ? (
                <InlineError>
                    <div data-testid="run-context-error">{contextError.message}</div>
                    <div data-testid="run-context-error-help" className="mt-1 text-xs">{contextError.help}</div>
                </InlineError>
            ) : null}
            {!contextError && status !== 'ready' && namespaces.length === 0 ? (
                <p data-testid="run-context-loading" className="text-sm text-muted-foreground" aria-live="polite">Restoring context…</p>
            ) : null}
            {status === 'ready' && namespaces.length === 0 ? (
                <p data-testid="run-context-empty" className="text-sm text-muted-foreground">
                    {query ? 'No context keys match the search.' : 'This run has no flow context yet.'}
                </p>
            ) : null}
            {namespaces.map((namespace) => (
                <section key={namespace.name} data-testid="run-context-namespace" data-namespace={namespace.name}>
                    <h4 className="pt-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">{namespace.name}</h4>
                    <ul className="divide-y divide-border">
                        {namespace.keys.map((row) => {
                            const last = row.history.at(-1)
                            const writes = row.history.length
                            const expanded = expandedKeys.has(row.key)
                            const expandable = writes > 1 || isLongValue(row.value)
                            return (
                                <li
                                    key={row.key}
                                    data-testid="run-context-row"
                                    data-context-key={row.key}
                                    className={cn('grid grid-cols-[minmax(8rem,14rem)_1fr] gap-3 py-1.5 text-sm', row.key === focusKey && 'bg-accent/40')}
                                >
                                    <code className="truncate text-foreground/80" title={row.key}>{row.key.replace(/^context\./, '')}</code>
                                    <div className="min-w-0 space-y-0.5">
                                        <p
                                            data-testid="run-context-row-value"
                                            className={expanded ? 'whitespace-pre-wrap break-words' : 'truncate'}
                                            title={expanded || row.value === null ? undefined : formatValue(row.value)}
                                        >
                                            {row.value === null
                                                ? <span className="text-muted-foreground">cleared</span>
                                                : expanded ? formatFullValue(row.value) : formatValue(row.value)}
                                        </p>
                                        <p className="text-xs text-muted-foreground">
                                            {last?.visit ? (
                                                <>{last.value === null ? 'cleared' : 'set'} by <VisitLink visit={last.visit} onSelect={onSelectVisit} /></>
                                            ) : last ? 'launch input' : 'set by the runtime'}
                                            {expandable ? (
                                                <>
                                                    {' · '}
                                                    <button
                                                        type="button"
                                                        data-testid="run-context-history-toggle"
                                                        aria-expanded={expanded}
                                                        onClick={() => toggle(row.key)}
                                                        className="hover:text-foreground"
                                                    >
                                                        {writes > 1
                                                            ? expanded ? 'hide history' : `written ${writes}×`
                                                            : expanded ? 'show less' : 'show full value'}
                                                    </button>
                                                </>
                                            ) : null}
                                        </p>
                                        {expanded && writes > 1 ? (
                                            <ol data-testid="run-context-history" className="space-y-0.5 border-l border-border pl-3 text-xs">
                                                {row.history.map((entry, index) => (
                                                    <li key={index} data-testid="run-context-history-entry" className="flex min-w-0 gap-2">
                                                        <span className="shrink-0 text-muted-foreground">
                                                            {entry.visit ? <VisitLink visit={entry.visit} onSelect={onSelectVisit} /> : 'launch'}:
                                                        </span>
                                                        <span className="min-w-0 whitespace-pre-wrap break-words">
                                                            {entry.value === null ? <span className="text-muted-foreground">cleared</span> : formatFullValue(entry.value)}
                                                        </span>
                                                    </li>
                                                ))}
                                            </ol>
                                        ) : null}
                                    </div>
                                </li>
                            )
                        })}
                    </ul>
                </section>
            ))}
            {systemKeys.length > 0 ? (
                <details data-testid="run-context-runtime-group" open={Boolean(query)} className="pt-2">
                    <summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground">
                        + {systemKeys.length} system {systemKeys.length === 1 ? 'key' : 'keys'}
                    </summary>
                    <ul className="mt-1 divide-y divide-border text-xs">
                        {systemKeys.map((key) => (
                            <li key={key} data-testid="run-context-system-row" className="grid grid-cols-[minmax(8rem,14rem)_1fr] gap-3 py-1">
                                <code className="truncate text-muted-foreground" title={key}>{key}</code>
                                <span className="truncate text-muted-foreground">
                                    {finalContext && Object.hasOwn(finalContext, key) ? formatValue(finalContext[key]) : '—'}
                                </span>
                            </li>
                        ))}
                    </ul>
                </details>
            ) : null}
        </article>
    )
}
