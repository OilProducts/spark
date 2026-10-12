import { FileText, RotateCw, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { CONVERSATION_ATTACHMENT_LIMITS, formatAttachmentSize } from '../hooks/useComposerAttachments'
import type { ComposerAttachment } from '../model/types'

/** The draft's pending files above the message box: thumbnails for images, chips for the rest. */
export function ComposerAttachments({
    attachments,
    onRetry,
    onRemove,
    isSending = false,
}: {
    attachments: ComposerAttachment[]
    onRetry: (localId: string) => void
    onRemove: (localId: string) => void
    /** A send referencing these uploads awaits the server's answer. */
    isSending?: boolean
}) {
    if (attachments.length === 0) {
        return null
    }
    const totalBytes = attachments.reduce((sum, entry) => sum + entry.file.size, 0)
    return (
        <div data-testid="composer-attachments" className="flex flex-col gap-1.5 px-3.5 pt-3">
            <ul aria-label="Attachments" className="flex flex-wrap gap-2">
                {attachments.map((entry) => {
                    const name = entry.file.name
                    return (
                        <li
                            key={entry.localId}
                            data-testid="composer-attachment"
                            data-status={entry.status}
                            className="flex max-w-full items-center gap-2 rounded-lg border border-border bg-muted/40 p-1.5 text-xs"
                        >
                            {entry.previewUrl ? (
                                <img src={entry.previewUrl} alt={name} className="size-10 shrink-0 rounded object-cover" />
                            ) : (
                                <FileText aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
                            )}
                            <span className="flex min-w-0 flex-col">
                                <span className="max-w-[12rem] truncate text-foreground" title={name}>{name}</span>
                                <span role="status" className={entry.status === 'failed' ? 'text-destructive' : 'text-muted-foreground'}>
                                    {entry.status === 'uploading'
                                        ? 'Uploading…'
                                        : entry.status === 'failed'
                                            ? entry.error || 'Upload failed.'
                                            : formatAttachmentSize(entry.file.size)}
                                </span>
                            </span>
                            {entry.status === 'failed' ? (
                                <Button type="button" variant="ghost" size="icon-xs" aria-label={`Retry uploading ${name}`} onClick={() => onRetry(entry.localId)}>
                                    <RotateCw />
                                </Button>
                            ) : null}
                            <Button type="button" variant="ghost" size="icon-xs" aria-label={`Remove ${name}`} disabled={isSending} onClick={() => onRemove(entry.localId)}>
                                <X />
                            </Button>
                        </li>
                    )
                })}
            </ul>
            <p data-testid="composer-attachment-limits" className="text-xs text-muted-foreground">
                {attachments.length} of {CONVERSATION_ATTACHMENT_LIMITS.maxFiles} files · {formatAttachmentSize(totalBytes)} of 50 MB per message · 20 MB per file
            </p>
        </div>
    )
}
