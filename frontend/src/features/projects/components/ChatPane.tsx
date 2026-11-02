import { useRef, useState, type ClipboardEvent, type DragEvent, type ReactNode } from 'react'
import { ArrowUp, Paperclip } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import type { useScrollFollow } from '../hooks/useScrollFollow'
import type { ComposerAttachment } from '../model/types'
import { ComposerAttachments } from './ComposerAttachments'

export interface ChatComposer {
    /** The textarea's accessible name. */
    label?: string
    placeholder: string
    draft: string
    onDraftChange: (value: string) => void
    onSubmit: () => void
    inputDisabled: boolean
    sendDisabled: boolean
    sendLabel: string
    attachments?: {
        items: ComposerAttachment[]
        onAdd: (files: File[]) => void
        onRetry: (localId: string) => void
        onRemove: (localId: string) => void
        isSending: boolean
        /** An upload is pending or failed, so the message cannot go yet. */
        isUploadBlocking: boolean
        disabled: boolean
    }
    /** Left of the composer's footer row. */
    footer?: ReactNode
    /** Before Send in the footer row. */
    controls?: ReactNode
}

/** A conversation's scrolling body, Jump to bottom and composer, shared by project chats and missions. */
export function ChatPane({ follow, scrolls, hasContent, children, notice, composer }: {
    follow: ReturnType<typeof useScrollFollow>
    /** Whether this pane's body is the scroller. false: the caller attaches follow.ref/onScroll to its own scroller. */
    scrolls: boolean
    /** Shows Jump to bottom when not pinned. */
    hasContent: boolean
    children: ReactNode
    /** Above the composer. */
    notice?: ReactNode
    /** Omitted: no composer. */
    composer?: ChatComposer
}) {
    const fileInputRef = useRef<HTMLInputElement>(null)
    const [isDraggingFiles, setIsDraggingFiles] = useState(false)
    const attachments = composer?.attachments
    const canAttach = Boolean(attachments) && !attachments?.disabled
    const hasMessage = Boolean(composer?.draft.trim()) || Boolean(attachments?.items.length)
    // Clipboard files are attached; any plain text alongside them still
    // pastes into the message box through the default paste.
    const onComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
        const files = Array.from(event.clipboardData.files)
        if (!canAttach || files.length === 0) return
        if (!Array.from(event.clipboardData.types).includes('text/plain')) event.preventDefault()
        attachments?.onAdd(files)
    }
    const dragHasFiles = (event: DragEvent) => Array.from(event.dataTransfer.types).includes('Files')
    const onComposerDragOver = (event: DragEvent<HTMLFormElement>) => {
        if (!canAttach || !dragHasFiles(event)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'copy'
        setIsDraggingFiles(true)
    }
    const onComposerDrop = (event: DragEvent<HTMLFormElement>) => {
        setIsDraggingFiles(false)
        if (!canAttach || !dragHasFiles(event)) return
        event.preventDefault()
        attachments?.onAdd(Array.from(event.dataTransfer.files))
    }
    return (
        <>
            <div
                {...(scrolls ? { ref: follow.ref, onScroll: follow.onScroll } : {})}
                data-testid="project-ai-conversation-body"
                className={cn('px-6 pb-3 pt-6', scrolls && 'min-h-0 flex-1 overflow-y-auto')}
            >
                <div className="mx-auto w-full max-w-3xl">{children}</div>
            </div>
            <div className="shrink-0 px-6 pb-5 pt-2">
                <div className="mx-auto flex w-full max-w-3xl flex-col gap-2">
                    {!follow.isPinned && hasContent ? (
                        <div className="flex justify-end">
                            <Button
                                type="button"
                                data-testid="project-ai-conversation-jump-to-bottom"
                                onClick={follow.scrollToBottom}
                                variant="outline"
                                size="xs"
                            >
                                Jump to bottom
                            </Button>
                        </div>
                    ) : null}
                    {notice}
                    {composer ? (
                        <form
                            data-testid="project-ai-conversation-composer"
                            onSubmit={(event) => {
                                event.preventDefault()
                                composer.onSubmit()
                            }}
                            onDragOver={onComposerDragOver}
                            onDragLeave={() => setIsDraggingFiles(false)}
                            onDrop={onComposerDrop}
                            className={cn(
                                'flex flex-col rounded-xl border border-input bg-background transition-[color,box-shadow] focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30',
                                isDraggingFiles && 'border-ring ring-2 ring-ring/30',
                            )}
                        >
                            {attachments ? (
                                <ComposerAttachments attachments={attachments.items} onRetry={attachments.onRetry} onRemove={attachments.onRemove} isSending={attachments.isSending} />
                            ) : null}
                            <Textarea
                                id="project-ai-conversation-input"
                                data-testid="project-ai-conversation-input"
                                value={composer.draft}
                                onChange={(event) => composer.onDraftChange(event.target.value)}
                                onKeyDown={(event) => {
                                    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                                        event.preventDefault()
                                        event.currentTarget.form?.requestSubmit()
                                    }
                                }}
                                onPaste={onComposerPaste}
                                aria-label={composer.label ?? 'Message'}
                                placeholder={composer.placeholder}
                                disabled={composer.inputDisabled}
                                className="max-h-[40vh] min-h-0 resize-none overflow-y-auto rounded-xl border-0 bg-transparent px-3.5 pb-1 pt-3 focus-visible:border-0 focus-visible:ring-0"
                                rows={2}
                            />
                            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2 pl-3.5 pr-2">
                                {composer.footer}
                                <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
                                    {attachments ? (
                                        <>
                                            <input
                                                ref={fileInputRef}
                                                data-testid="project-ai-conversation-file-input"
                                                type="file"
                                                multiple
                                                hidden
                                                onChange={(event) => {
                                                    attachments.onAdd(Array.from(event.target.files ?? []))
                                                    event.target.value = ''
                                                }}
                                            />
                                            <Button
                                                type="button"
                                                data-testid="project-ai-conversation-attach-button"
                                                variant="ghost"
                                                size="icon-sm"
                                                aria-label="Attach files"
                                                title="Attach files (up to 10, 20 MB each, 50 MB per message)"
                                                disabled={!canAttach}
                                                onClick={() => fileInputRef.current?.click()}
                                            >
                                                <Paperclip />
                                            </Button>
                                        </>
                                    ) : null}
                                    {composer.controls}
                                    <Button
                                        data-testid="project-ai-conversation-send-button"
                                        type="submit"
                                        size="icon-sm"
                                        aria-label={composer.sendLabel}
                                        title={composer.sendLabel}
                                        disabled={!hasMessage || Boolean(attachments?.isUploadBlocking) || composer.sendDisabled}
                                    >
                                        <ArrowUp />
                                    </Button>
                                </div>
                            </div>
                        </form>
                    ) : null}
                </div>
            </div>
        </>
    )
}
