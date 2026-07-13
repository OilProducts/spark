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
}

interface ArtifactGroup {
    key: string
    label: string
    entries: ArtifactListEntry[]
}

/**
 * Group by origin: per-node logs are the raw material behind a node's
 * transcript, flow snapshots record provenance, everything else is a
 * run-level file.
 */
function groupArtifacts(entries: ArtifactListEntry[]): ArtifactGroup[] {
    const nodeGroups = new Map<string, ArtifactListEntry[]>()
    const flowEntries: ArtifactListEntry[] = []
    const runEntries: ArtifactListEntry[] = []
    for (const entry of entries) {
        const nodeMatch = /^logs\/([^/]+)\/.+/.exec(entry.path)
        if (nodeMatch) {
            const node = nodeMatch[1]
            nodeGroups.set(node, [...(nodeGroups.get(node) ?? []), entry])
            continue
        }
        if (entry.path.startsWith('artifacts/flow/')) {
            flowEntries.push(entry)
            continue
        }
        runEntries.push(entry)
    }
    const groups: ArtifactGroup[] = []
    for (const [node, nodeEntries] of [...nodeGroups.entries()].sort(([a], [b]) => a.localeCompare(b))) {
        groups.push({ key: `node-${node}`, label: `Node · ${node}`, entries: nodeEntries })
    }
    if (flowEntries.length > 0) {
        groups.push({ key: 'flow', label: 'Flow', entries: flowEntries })
    }
    if (runEntries.length > 0) {
        groups.push({ key: 'run', label: 'Run files', entries: runEntries })
    }
    return groups
}

function displayPath(group: ArtifactGroup, path: string): string {
    if (group.key.startsWith('node-')) {
        return path.replace(/^logs\/[^/]+\//, '')
    }
    if (group.key === 'flow') {
        return path.replace(/^artifacts\/flow\//, '')
    }
    return path
}

function formatSize(sizeBytes: number): string {
    if (sizeBytes < 1024) {
        return `${sizeBytes} B`
    }
    if (sizeBytes < 1024 * 1024) {
        return `${(sizeBytes / 1024).toFixed(1)} KB`
    }
    return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`
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
}: RunArtifactsCardProps) {
    const groups = groupArtifacts(artifactEntries)
    return (
        <Card data-testid="run-artifact-panel" className="gap-3 py-4">
            <CardHeader className="gap-2 px-4">
                <div className="flex items-center justify-end">
                    <Button
                        onClick={onRefresh}
                        data-testid="run-artifact-refresh-button"
                        variant="outline"
                        size="xs"
                        className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                    >
                        {isLoading ? 'Refreshing…' : 'Refresh'}
                    </Button>
                </div>
            </CardHeader>
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
                        {groups.length > 0 ? (
                            groups.map((group) => (
                                <section key={group.key} data-testid={`run-artifact-group-${group.key}`} className="space-y-1">
                                    <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                        {group.label}
                                    </h4>
                                    <div className="overflow-hidden rounded-md border border-border/80">
                                        <table data-testid="run-artifact-table" className="w-full border-collapse text-sm">
                                            <tbody>
                                                {group.entries.map((artifact) => (
                                                    <tr key={artifact.path} data-testid="run-artifact-row" className="border-t border-border/70 first:border-t-0">
                                                        <td
                                                            className="break-all px-3 py-1.5 font-mono text-xs text-foreground"
                                                            title={artifact.path}
                                                        >
                                                            {displayPath(group, artifact.path)}
                                                        </td>
                                                        <td className="w-20 whitespace-nowrap px-2 py-1.5 text-right font-mono text-xs text-muted-foreground">
                                                            {formatSize(artifact.size_bytes)}
                                                        </td>
                                                        <td className="w-36 px-2 py-1.5">
                                                            <div className="flex items-center justify-end gap-1.5">
                                                                <Button
                                                                    type="button"
                                                                    data-testid="run-artifact-view-button"
                                                                    disabled={!artifact.viewable}
                                                                    onClick={() => {
                                                                        void onViewArtifact(artifact)
                                                                    }}
                                                                    variant="outline"
                                                                    size="xs"
                                                                    className="h-6 px-2 text-[11px] text-muted-foreground hover:text-foreground"
                                                                >
                                                                    View
                                                                </Button>
                                                                <a
                                                                    data-testid="run-artifact-download-link"
                                                                    href={artifactDownloadHref(artifact.path) || undefined}
                                                                    download={artifact.path.split('/').pop() || 'artifact'}
                                                                    className="inline-flex h-6 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                                                >
                                                                    Download
                                                                </a>
                                                            </div>
                                                        </td>
                                                    </tr>
                                                ))}
                                            </tbody>
                                        </table>
                                    </div>
                                </section>
                            ))
                        ) : (
                            <Empty data-testid="run-artifact-empty" className="text-sm text-muted-foreground">
                                <EmptyHeader>
                                    <EmptyDescription>
                                        No run artifacts are available yet.
                                    </EmptyDescription>
                                </EmptyHeader>
                            </Empty>
                        )}
                        <div data-testid="run-artifact-viewer" className="rounded-md border border-border/80 bg-muted/30 p-3">
                            <div className="mb-2 text-xs text-muted-foreground">
                                {selectedArtifactEntry ? `Preview: ${selectedArtifactEntry.path}` : 'Select a viewable artifact to preview.'}
                            </div>
                            {selectedArtifactEntry?.context_capture_kind === 'codex_turn_input' && (
                                <div data-testid="run-artifact-codex-context-note" className="mb-2 text-xs text-muted-foreground">
                                    Codex may add internal instructions that are not observable by Spark.
                                </div>
                            )}
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
        </Card>
    )
}
