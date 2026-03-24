import type { GraphvizErrorState } from '../model/shared'

interface RunGraphvizCardProps {
    isGraphvizLoading: boolean
    graphvizError: GraphvizErrorState | null
    graphvizViewerSrc: string | null
    runId: string
    onRefresh: () => void
}

export function RunGraphvizCard({
    isGraphvizLoading,
    graphvizError,
    graphvizViewerSrc,
    runId,
    onRefresh,
}: RunGraphvizCardProps) {
    return (
        <div data-testid="run-graphviz-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
            <div className="mb-3 flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Graphviz Render</h3>
                <button
                    onClick={onRefresh}
                    data-testid="run-graphviz-refresh-button"
                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                >
                    {isGraphvizLoading ? 'Refreshing…' : 'Refresh'}
                </button>
            </div>
            <div data-testid="run-graphviz-viewer" className="rounded-md border border-border/80 bg-muted/30 p-3">
                {isGraphvizLoading && (
                    <div data-testid="run-graphviz-viewer-loading" className="text-xs text-muted-foreground">
                        Loading graph visualization...
                    </div>
                )}
                {!isGraphvizLoading && graphvizError && (
                    <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                        <div data-testid="run-graphviz-viewer-error">{graphvizError.message}</div>
                        <div data-testid="run-graphviz-viewer-error-help" className="text-xs text-destructive/90">
                            {graphvizError.help}
                        </div>
                    </div>
                )}
                {!isGraphvizLoading && !graphvizError && graphvizViewerSrc && (
                    <img
                        data-testid="run-graphviz-viewer-image"
                        src={graphvizViewerSrc}
                        alt={`Graphviz render for run ${runId}`}
                        className="w-full rounded-md border border-border/70 bg-background"
                    />
                )}
            </div>
        </div>
    )
}
