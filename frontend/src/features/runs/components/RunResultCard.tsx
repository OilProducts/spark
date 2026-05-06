import type { PipelineResultResponse } from '@/lib/attractorClient'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'

interface RunResultCardProps {
    result: PipelineResultResponse | null
    resultError: string | null
    isLoading: boolean
    onRefresh: () => void
    onViewSource: (artifactPath: string) => void
}

export function RunResultCard({
    result,
    resultError,
    isLoading,
    onRefresh,
    onViewSource,
}: RunResultCardProps) {
    const sourcePath = result?.source_artifact_path || ''
    return (
        <Card data-testid="run-result-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Result</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            {result?.display_mode === 'summary' ? 'Summarized output' : 'Final output'}
                            {result?.source_node_id ? ` from ${result.source_node_id}` : ''}
                        </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                        {sourcePath ? (
                            <Button
                                type="button"
                                data-testid="run-result-source-button"
                                onClick={() => onViewSource(sourcePath)}
                                variant="outline"
                                size="xs"
                                className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                            >
                                Source
                            </Button>
                        ) : null}
                        <Button
                            type="button"
                            data-testid="run-result-refresh-button"
                            onClick={onRefresh}
                            variant="outline"
                            size="xs"
                            className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                        >
                            {isLoading ? 'Refreshing...' : 'Refresh'}
                        </Button>
                    </div>
                </div>
            </CardHeader>
            <CardContent className="space-y-3 px-4">
                {resultError ? (
                    <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                        <AlertDescription data-testid="run-result-error" className="text-inherit">
                            {resultError}
                        </AlertDescription>
                    </Alert>
                ) : null}
                {!resultError && (!result || result.state === 'pending') ? (
                    <Alert
                        data-testid="run-result-pending"
                        className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                    >
                        <AlertDescription className="text-inherit">
                            Result will be available after the run reaches a terminal state.
                        </AlertDescription>
                    </Alert>
                ) : null}
                {!resultError && result?.state === 'unavailable' ? (
                    <Alert
                        data-testid="run-result-unavailable"
                        className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                    >
                        <AlertDescription className="text-inherit">
                            No result source was found for this run.
                        </AlertDescription>
                    </Alert>
                ) : null}
                {!resultError && result?.state === 'error' ? (
                    <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                        <AlertDescription data-testid="run-result-resolution-error" className="text-inherit">
                            {result.error || 'Result resolution failed.'}
                        </AlertDescription>
                    </Alert>
                ) : null}
                {!resultError && result?.state === 'ready' ? (
                    <div data-testid="run-result-body" className="rounded-md border border-border/80 bg-muted/20 p-3">
                        <ProjectConversationMarkdown content={result.body_markdown || ''} />
                        {result.summary_error ? (
                            <p data-testid="run-result-summary-error" className="mt-3 text-xs text-amber-800">
                                Summary unavailable: {result.summary_error}
                            </p>
                        ) : null}
                    </div>
                ) : null}
            </CardContent>
        </Card>
    )
}
