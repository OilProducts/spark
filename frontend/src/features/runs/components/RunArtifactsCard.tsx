import type {
    ArtifactErrorState,
    ArtifactListEntry,
} from '../model/shared'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunArtifactsCardProps {
    isLoading: boolean
    status: 'idle' | 'loading' | 'ready' | 'error'
    artifactError: ArtifactErrorState | null
    artifactEntries: ArtifactListEntry[]
    selectedArtifactEntry: ArtifactListEntry | null
    isArtifactViewerLoading: boolean
    artifactViewerError: string | null
    artifactViewerPayload: string | null
    showPartialRunArtifactNote: boolean
    missingCoreArtifacts: string[]
    onRefresh: () => void
    onViewArtifact: (artifact: ArtifactListEntry) => void | Promise<void>
    artifactDownloadHref: (artifactPath: string) => string | null
    collapsed: boolean
    onCollapsedChange: (collapsed: boolean) => void
}

export function RunArtifactsCard({
    isLoading,
    status,
    artifactError,
    artifactEntries,
    selectedArtifactEntry,
    isArtifactViewerLoading,
    artifactViewerError,
    artifactViewerPayload,
    showPartialRunArtifactNote,
    missingCoreArtifacts,
    onRefresh,
    onViewArtifact,
    artifactDownloadHref,
    collapsed,
    onCollapsedChange,
}: RunArtifactsCardProps) {
    return (
        <Card data-testid="run-artifact-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Artifacts</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Browse generated files, inspect previews, and download outputs.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <Button
                            onClick={onRefresh}
                            data-testid="run-artifact-refresh-button"
                            variant="outline"
                            size="xs"
                            className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                        >
                            {isLoading ? 'Refreshing…' : 'Refresh'}
                        </Button>
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-artifact-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
            {artifactError && (
                <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
                    <AlertDescription className="space-y-1 text-inherit">
                        <div data-testid="run-artifact-error">{artifactError.message}</div>
                        <div data-testid="run-artifact-error-help" className="text-xs text-destructive/90">
                            {artifactError.help}
                        </div>
                    </AlertDescription>
                </Alert>
            )}
            {!artifactError && status !== 'ready' ? (
                <Alert
                    data-testid="run-artifact-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        Restoring artifacts…
                    </AlertDescription>
                </Alert>
            ) : null}
            {!artifactError && status === 'ready' && (
                <div className="space-y-3">
                    {showPartialRunArtifactNote && (
                        <div
                            data-testid="run-artifact-partial-run-note"
                            className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-800"
                        >
                            <div>This run may be partial or artifacts may have been pruned.</div>
                            {missingCoreArtifacts.length > 0 && (
                                <div className="mt-1">
                                    Missing expected files: {missingCoreArtifacts.join(', ')}.
                                </div>
                            )}
                        </div>
                    )}
                    <div className="overflow-hidden rounded-md border border-border/80">
                        <table data-testid="run-artifact-table" className="w-full table-fixed border-collapse text-sm">
                            <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                                <tr>
                                    <th className="w-1/2 px-3 py-2 font-semibold">Path</th>
                                    <th className="w-28 px-3 py-2 font-semibold">Type</th>
                                    <th className="w-28 px-3 py-2 font-semibold">Size</th>
                                    <th className="px-3 py-2 font-semibold">Actions</th>
                                </tr>
                            </thead>
                            <tbody>
                                {artifactEntries.length > 0 ? (
                                    artifactEntries.map((artifact) => (
                                        <tr key={artifact.path} data-testid="run-artifact-row" className="border-t border-border/70 align-top">
                                            <td className="break-all px-3 py-2 font-mono text-xs text-foreground">{artifact.path}</td>
                                            <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{artifact.media_type}</td>
                                            <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{artifact.size_bytes.toLocaleString()}</td>
                                            <td className="px-3 py-2">
                                                <div className="flex items-center gap-2">
                                                    <Button
                                                        type="button"
                                                        data-testid="run-artifact-view-button"
                                                        disabled={!artifact.viewable}
                                                        onClick={() => {
                                                            void onViewArtifact(artifact)
                                                        }}
                                                        variant="outline"
                                                        size="xs"
                                                        className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                                                    >
                                                        View
                                                    </Button>
                                                    <a
                                                        data-testid="run-artifact-download-link"
                                                        href={artifactDownloadHref(artifact.path) || undefined}
                                                        download={artifact.path.split('/').pop() || 'artifact'}
                                                        className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                                    >
                                                        Download
                                                    </a>
                                                </div>
                                            </td>
                                        </tr>
                                    ))
                                ) : (
                                    <tr>
                                        <td colSpan={4} className="px-3 py-4">
                                            <Empty data-testid="run-artifact-empty" className="text-sm text-muted-foreground">
                                                <EmptyHeader>
                                                    <EmptyDescription>
                                                        No run artifacts are available yet.
                                                    </EmptyDescription>
                                                </EmptyHeader>
                                            </Empty>
                                        </td>
                                    </tr>
                                )}
                            </tbody>
                        </table>
                    </div>
                    <div data-testid="run-artifact-viewer" className="rounded-md border border-border/80 bg-muted/30 p-3">
                        <div className="mb-2 text-xs text-muted-foreground">
                            {selectedArtifactEntry ? `Preview: ${selectedArtifactEntry.path}` : 'Select a viewable artifact to preview.'}
                        </div>
                        {isArtifactViewerLoading && (
                            <div data-testid="run-artifact-viewer-loading" className="text-xs text-muted-foreground">
                                Loading artifact preview...
                            </div>
                        )}
                        {!isArtifactViewerLoading && artifactViewerError && (
                            <div data-testid="run-artifact-viewer-error" className="text-xs text-destructive">
                                {artifactViewerError}
                            </div>
                        )}
                        {!isArtifactViewerLoading && !artifactViewerError && artifactViewerPayload && (
                            <pre
                                data-testid="run-artifact-viewer-payload"
                                className="max-h-60 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-background px-2 py-2 font-mono text-xs text-foreground"
                            >
                                {artifactViewerPayload}
                            </pre>
                        )}
                    </div>
                </div>
            )}
                </CardContent>
            ) : null}
        </Card>
    )
}
