import type {
    CheckpointErrorState,
    CheckpointResponse,
} from '../model/shared'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunCheckpointCardProps {
    isLoading: boolean
    status: 'idle' | 'loading' | 'ready' | 'error'
    checkpointError: CheckpointErrorState | null
    checkpointData: CheckpointResponse['checkpoint'] | null
    checkpointCurrentNode: string
    checkpointCompletedNodes: string
    checkpointRetryCounters: string
    onRefresh: () => void
    collapsed: boolean
    onCollapsedChange: (collapsed: boolean) => void
}

export function RunCheckpointCard({
    isLoading,
    status,
    checkpointError,
    checkpointData,
    checkpointCurrentNode,
    checkpointCompletedNodes,
    checkpointRetryCounters,
    onRefresh,
    collapsed,
    onCollapsedChange,
}: RunCheckpointCardProps) {
    return (
        <Card data-testid="run-checkpoint-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Checkpoint</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Latest persisted runtime position and retry counters.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <Button
                            onClick={onRefresh}
                            data-testid="run-checkpoint-refresh-button"
                            variant="outline"
                            size="xs"
                            className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                        >
                            {isLoading ? 'Refreshing…' : 'Refresh'}
                        </Button>
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-checkpoint-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
            {!checkpointError && status !== 'ready' ? (
                <Alert
                    data-testid="run-checkpoint-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        Restoring checkpoint…
                    </AlertDescription>
                </Alert>
            ) : null}
            {checkpointError && (
                <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                    <AlertDescription className="space-y-1 text-inherit">
                        <div data-testid="run-checkpoint-error">{checkpointError.message}</div>
                        <div data-testid="run-checkpoint-error-help" className="text-xs text-destructive/90">
                            {checkpointError.help}
                        </div>
                    </AlertDescription>
                </Alert>
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
            {!checkpointError && status === 'ready' && !checkpointData ? (
                <Alert
                    data-testid="run-checkpoint-empty"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        No checkpoint data is available for this run yet.
                    </AlertDescription>
                </Alert>
            ) : null}
                </CardContent>
            ) : null}
        </Card>
    )
}
