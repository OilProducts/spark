import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import type { ArtifactListEntry } from '../model/shared'

interface RunArtifactViewerProps {
    entry: ArtifactListEntry | { path: string } | null
    open: boolean
    onOpenChange: (open: boolean) => void
    isLoading: boolean
    error: string | null
    payload: string | null
    downloadHref: string | null
}

/** One run file, opened from a visit's files or the status item's outputs. */
export function RunArtifactViewer({
    entry,
    open,
    onOpenChange,
    isLoading,
    error,
    payload,
    downloadHref,
}: RunArtifactViewerProps) {
    return (
        <Dialog open={open && Boolean(entry)} onOpenChange={onOpenChange}>
            <DialogContent data-testid="run-artifact-viewer" className="flex max-h-[85vh] flex-col gap-3 overflow-hidden sm:max-w-3xl">
                <div className="flex items-baseline gap-3 pr-8">
                    <DialogTitle className="min-w-0 truncate font-mono text-sm font-normal" title={entry?.path}>
                        {entry?.path}
                    </DialogTitle>
                    {downloadHref && entry ? (
                        <a
                            data-testid="run-artifact-download-link"
                            href={downloadHref}
                            download={entry.path.split('/').pop() || 'artifact'}
                            className="ml-auto shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                        >
                            Download
                        </a>
                    ) : null}
                </div>
                {entry && 'context_capture_kind' in entry && entry.context_capture_kind === 'codex_turn_input' ? (
                    <p data-testid="run-artifact-codex-context-note" className="text-xs text-muted-foreground">
                        Codex may add instructions Spark can't see.
                    </p>
                ) : null}
                {isLoading ? (
                    <p data-testid="run-artifact-viewer-loading" className="text-xs text-muted-foreground">Loading…</p>
                ) : error ? (
                    <p data-testid="run-artifact-viewer-error" className="text-xs text-destructive">{error}</p>
                ) : (
                    <pre
                        data-testid="run-artifact-viewer-payload"
                        className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-foreground"
                    >
                        {payload}
                    </pre>
                )}
            </DialogContent>
        </Dialog>
    )
}
