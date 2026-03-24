import type {
    CheckpointErrorState,
    CheckpointResponse,
} from '@/components/runs/shared'

interface RunCheckpointCardProps {
    isLoading: boolean
    checkpointError: CheckpointErrorState | null
    checkpointData: CheckpointResponse['checkpoint'] | null
    checkpointCurrentNode: string
    checkpointCompletedNodes: string
    checkpointRetryCounters: string
    onRefresh: () => void
}

export function RunCheckpointCard({
    isLoading,
    checkpointError,
    checkpointData,
    checkpointCurrentNode,
    checkpointCompletedNodes,
    checkpointRetryCounters,
    onRefresh,
}: RunCheckpointCardProps) {
    return (
        <div data-testid="run-checkpoint-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
            <div className="mb-3 flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Checkpoint</h3>
                <button
                    onClick={onRefresh}
                    data-testid="run-checkpoint-refresh-button"
                    className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                >
                    {isLoading ? 'Refreshing…' : 'Refresh'}
                </button>
            </div>
            {checkpointError && (
                <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    <div data-testid="run-checkpoint-error">{checkpointError.message}</div>
                    <div data-testid="run-checkpoint-error-help" className="text-xs text-destructive/90">
                        {checkpointError.help}
                    </div>
                </div>
            )}
            {!checkpointError && checkpointData && (
                <div className="space-y-3">
                    <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-3">
                        <div data-testid="run-checkpoint-current-node">
                            <span className="font-medium">Current Node:</span> {checkpointCurrentNode}
                        </div>
                        <div data-testid="run-checkpoint-completed-nodes">
                            <span className="font-medium">Completed Nodes:</span> {checkpointCompletedNodes}
                        </div>
                        <div data-testid="run-checkpoint-retry-counters">
                            <span className="font-medium">Retry Counters:</span> {checkpointRetryCounters}
                        </div>
                    </div>
                    <pre
                        data-testid="run-checkpoint-payload"
                        className="max-h-60 overflow-auto rounded-md border border-border/80 bg-muted/40 p-3 text-xs text-foreground"
                    >
                        {JSON.stringify(checkpointData, null, 2)}
                    </pre>
                </div>
            )}
        </div>
    )
}
