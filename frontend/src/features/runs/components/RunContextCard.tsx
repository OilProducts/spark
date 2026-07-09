import type { ContextErrorState, RunContextRow } from '../model/shared'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'

interface RunContextCardProps {
    runId: string
    isLoading: boolean
    status: 'idle' | 'loading' | 'ready' | 'error'
    contextError: ContextErrorState | null
    filteredContextRows: RunContextRow[]
    searchQuery: string
    contextCopyStatus: string
    contextExportHref: string | null
    onRefresh: () => void
    onCopy: () => void
    onSearchQueryChange: (query: string) => void
}

const isRuntimeKey = (key: string): boolean => (
    key.startsWith('_attractor.')
    || key.startsWith('execution_')
    || key === 'current_node'
)

function ContextRows({ rows }: { rows: RunContextRow[] }) {
    return (
        <>
            {rows.map((row) => (
                <tr key={row.key} data-testid="run-context-row" className="border-t border-border/70 align-top">
                    <td className="break-all px-3 py-2 font-mono text-xs text-foreground">{row.key}</td>
                    <td className="px-3 py-2 font-mono text-xs text-foreground">
                        <span
                            data-testid="run-context-row-type"
                            className="mr-2 inline-flex rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                        >
                            {row.valueType}
                        </span>
                        {row.renderKind === 'structured' ? (
                            <div data-testid="run-context-row-value">
                                <pre
                                    data-testid="run-context-row-value-structured"
                                    className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded border border-border/70 bg-muted/40 px-2 py-1"
                                >
                                    {row.renderedValue}
                                </pre>
                            </div>
                        ) : (
                            <span data-testid="run-context-row-value" className="break-all">
                                <span data-testid="run-context-row-value-scalar">{row.renderedValue}</span>
                            </span>
                        )}
                    </td>
                </tr>
            ))}
        </>
    )
}

export function RunContextCard({
    isLoading,
    status,
    contextError,
    filteredContextRows,
    searchQuery,
    contextCopyStatus,
    contextExportHref,
    onRefresh,
    onCopy,
    onSearchQueryChange,
}: RunContextCardProps) {
    // Flow data — the keys nodes write for each other — leads; runtime
    // plumbing collapses below it.
    const flowRows = filteredContextRows.filter((row) => !isRuntimeKey(row.key))
    const runtimeRows = filteredContextRows.filter((row) => isRuntimeKey(row.key))
    const isSearching = searchQuery.trim().length > 0

    return (
        <Card data-testid="run-context-panel" className="gap-3 py-4">
            <CardHeader className="gap-2 px-4">
                <div className="flex flex-wrap items-center gap-2">
                    <Input
                        value={searchQuery}
                        onChange={(event) => onSearchQueryChange(event.target.value)}
                        placeholder="Search context key or value..."
                        data-testid="run-context-search-input"
                        className="h-8 max-w-md flex-1 text-sm"
                    />
                    <Button
                        onClick={onRefresh}
                        data-testid="run-context-refresh-button"
                        variant="outline"
                        size="xs"
                        className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                    >
                        {isLoading ? 'Refreshing…' : 'Refresh'}
                    </Button>
                    <Button
                        type="button"
                        onClick={onCopy}
                        data-testid="run-context-copy-button"
                        variant="outline"
                        size="xs"
                        className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                    >
                        Copy JSON
                    </Button>
                    {contextExportHref ? (
                        <a
                            data-testid="run-context-export-button"
                            href={contextExportHref}
                            download="run-context.json"
                            className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                        >
                            Export JSON
                        </a>
                    ) : null}
                </div>
            </CardHeader>
            <CardContent className="space-y-3 px-4">
                {contextCopyStatus && (
                    <div data-testid="run-context-copy-status" className="text-xs text-muted-foreground">
                        {contextCopyStatus}
                    </div>
                )}
                {contextError && (
                    <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                        <AlertDescription className="space-y-1 text-inherit">
                            <div data-testid="run-context-error">{contextError.message}</div>
                            <div data-testid="run-context-error-help" className="text-xs text-destructive/90">
                                {contextError.help}
                            </div>
                        </AlertDescription>
                    </Alert>
                )}
                {!contextError && status !== 'ready' ? (
                    <Alert
                        data-testid="run-context-loading"
                        className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                    >
                        <AlertDescription className="text-inherit">
                            Restoring context…
                        </AlertDescription>
                    </Alert>
                ) : null}
                {!contextError && status === 'ready' && (
                    <div className="space-y-3">
                        <div className="overflow-hidden rounded-md border border-border/80">
                            <table data-testid="run-context-table" className="w-full table-fixed border-collapse text-sm">
                                <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                                    <tr>
                                        <th className="w-2/5 px-3 py-2 font-semibold">Key</th>
                                        <th className="px-3 py-2 font-semibold">Value</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {flowRows.length > 0 ? (
                                        <ContextRows rows={flowRows} />
                                    ) : (
                                        <tr>
                                            <td data-testid="run-context-empty" colSpan={2} className="px-3 py-4 text-sm text-muted-foreground">
                                                {isSearching
                                                    ? 'No flow context entries match the current search.'
                                                    : 'No flow context entries are available for this run yet.'}
                                            </td>
                                        </tr>
                                    )}
                                </tbody>
                            </table>
                        </div>
                        {runtimeRows.length > 0 ? (
                            <details data-testid="run-context-runtime-group" open={isSearching}>
                                <summary className="cursor-pointer select-none text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                    Runtime internals ({runtimeRows.length})
                                </summary>
                                <div className="mt-2 overflow-hidden rounded-md border border-border/80">
                                    <table className="w-full table-fixed border-collapse text-sm">
                                        <tbody>
                                            <ContextRows rows={runtimeRows} />
                                        </tbody>
                                    </table>
                                </div>
                            </details>
                        ) : null}
                    </div>
                )}
            </CardContent>
        </Card>
    )
}
