import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
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
    // A Markdown file (a prompt, a response, the result) reads formatted; its source is a click away.
    const [showSource, setShowSource] = useState(false)
    const formatted = Boolean(entry?.path.toLowerCase().endsWith('.md')) && !showSource
    return (
        <Dialog open={open && Boolean(entry)} onOpenChange={onOpenChange}>
            <DialogContent data-testid="run-artifact-viewer" className="flex max-h-[85vh] flex-col gap-3 overflow-hidden sm:max-w-3xl">
                <div className="flex items-center gap-3 pr-8">
                    <DialogTitle className="min-w-0 truncate font-mono text-sm font-normal" title={entry?.path}>
                        {entry?.path}
                    </DialogTitle>
                    <span className="flex-1" />
                    {entry?.path.toLowerCase().endsWith('.md') && payload?.trim() ? (
                        <Button
                            data-testid="run-artifact-source-toggle"
                            type="button"
                            size="xs"
                            variant="ghost"
                            aria-pressed={showSource}
                            onClick={() => setShowSource((value) => !value)}
                        >
                            Show source
                        </Button>
                    ) : null}
                    {downloadHref && entry ? (
                        <a
                            data-testid="run-artifact-download-link"
                            href={downloadHref}
                            download={entry.path.split('/').pop() || 'artifact'}
                            className="shrink-0 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
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
                ) : !payload?.trim() ? (
                    <p data-testid="run-artifact-viewer-empty" className="text-xs text-muted-foreground">This file is empty.</p>
                ) : formatted ? (
                    <div data-testid="run-artifact-viewer-payload" className="min-h-0 flex-1 overflow-auto text-sm">
                        <ProjectConversationMarkdown content={payload} keepLineBreaks />
                    </div>
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
