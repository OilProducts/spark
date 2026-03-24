import type { ContextErrorState, RunContextRow } from '@/components/runs/shared'

interface RunContextCardProps {
    isLoading: boolean
    contextError: ContextErrorState | null
    contextCopyStatus: string
    searchQuery: string
    filteredContextRows: RunContextRow[]
    contextExportHref: string | null
    runId: string
    onRefresh: () => void | Promise<void>
    onCopy: () => void | Promise<void>
    onSearchQueryChange: (value: string) => void
}

export function RunContextCard({
    isLoading,
    contextError,
    contextCopyStatus,
    searchQuery,
    filteredContextRows,
    contextExportHref,
    runId,
    onRefresh,
    onCopy,
    onSearchQueryChange,
}: RunContextCardProps) {
    return (
        <div data-testid="run-context-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
            <div className="mb-3 flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Context</h3>
                <div className="flex items-center gap-2">
                    <button
                        onClick={onRefresh}
                        data-testid="run-context-refresh-button"
                        className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                    >
                        {isLoading ? 'Refreshing…' : 'Refresh'}
                    </button>
                    <button
                        type="button"
                        onClick={onCopy}
                        data-testid="run-context-copy-button"
                        className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                    >
                        Copy JSON
                    </button>
                    <a
                        data-testid="run-context-export-button"
                        href={contextExportHref || undefined}
                        download={`run-context-${runId}.json`}
                        onClick={(event) => {
                            if (!contextExportHref) {
                                event.preventDefault()
                            }
                        }}
                        className={`inline-flex h-7 items-center rounded-md border px-2 text-[11px] font-medium ${
                            contextExportHref
                                ? 'border-border text-muted-foreground hover:text-foreground'
                                : 'cursor-not-allowed border-border/60 text-muted-foreground/50'
                        }`}
                    >
                        Export JSON
                    </a>
                </div>
            </div>
            {contextCopyStatus && (
                <div data-testid="run-context-copy-status" className="mb-3 text-xs text-muted-foreground">
                    {contextCopyStatus}
                </div>
            )}
            <div className="mb-3">
                <input
                    value={searchQuery}
                    onChange={(event) => onSearchQueryChange(event.target.value)}
                    placeholder="Search context key or value..."
                    data-testid="run-context-search-input"
                    className="h-9 w-full rounded-md border border-border bg-background px-3 text-sm text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                />
            </div>
            {contextError && (
                <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    <div data-testid="run-context-error">{contextError.message}</div>
                    <div data-testid="run-context-error-help" className="text-xs text-destructive/90">
                        {contextError.help}
                    </div>
                </div>
            )}
            {!contextError && (
                <div className="overflow-hidden rounded-md border border-border/80">
                    <table data-testid="run-context-table" className="w-full table-fixed border-collapse text-sm">
                        <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                            <tr>
                                <th className="w-2/5 px-3 py-2 font-semibold">Key</th>
                                <th className="px-3 py-2 font-semibold">Value</th>
                            </tr>
                        </thead>
                        <tbody>
                            {filteredContextRows.length > 0 ? (
                                filteredContextRows.map((row) => (
                                    <tr key={row.key} data-testid="run-context-row" className="border-t border-border/70 align-top">
                                        <td className="px-3 py-2 font-mono text-xs text-foreground">{row.key}</td>
                                        <td className="space-x-2 px-3 py-2 font-mono text-xs text-foreground break-all">
                                            <span
                                                data-testid="run-context-row-type"
                                                className="inline-flex rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                                            >
                                                {row.valueType}
                                            </span>
                                            {row.renderKind === 'structured' ? (
                                                <div data-testid="run-context-row-value">
                                                    <pre
                                                        data-testid="run-context-row-value-structured"
                                                        className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-muted/40 px-2 py-1"
                                                    >
                                                        {row.renderedValue}
                                                    </pre>
                                                </div>
                                            ) : (
                                                <span data-testid="run-context-row-value">
                                                    <span data-testid="run-context-row-value-scalar">{row.renderedValue}</span>
                                                </span>
                                            )}
                                        </td>
                                    </tr>
                                ))
                            ) : (
                                <tr>
                                    <td data-testid="run-context-empty" colSpan={2} className="px-3 py-4 text-sm text-muted-foreground">
                                        No context entries match the current search.
                                    </td>
                                </tr>
                            )}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    )
}
