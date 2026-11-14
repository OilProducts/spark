import { useEffect } from 'react'

import { Button } from '@/components/ui/button'
import { retryLastSaveContent } from '@/lib/flowPersistence'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'

const SAVED_SHOWN_MS = 2000

/**
 * The flow's save state, in a corner of the canvas below the toolbar, so it never covers or moves
 * the actions it gates: "Saving…", a brief "Saved", or why the edits aren't saved yet. A failed save
 * shows its reason and the way out as text and buttons, not only on hover: Retry when trying again
 * can help, and Reload flow after a conflict.
 */
export function EditorSaveStatus({ className, onReload }: { className?: string; onReload?: () => void }) {
    const saveState = useStore((state) => state.saveState)
    const saveStateVersion = useStore((state) => state.saveStateVersion)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const resetSaveState = useStore((state) => state.resetSaveState)

    // "Saved" shows briefly, then clears.
    useEffect(() => {
        if (saveState !== 'saved') return
        const timer = window.setTimeout(resetSaveState, SAVED_SHOWN_MS)
        return () => window.clearTimeout(timer)
    }, [resetSaveState, saveState, saveStateVersion])

    const remediation = resolveSaveRemediation(saveState, saveErrorKind)
    // A flow saves only once it validates; the Validation panel lists what to fix.
    const blockedByValidation = saveState === 'error' && saveErrorKind === 'validation_error'
    const failed = saveState === 'conflict' || (saveState === 'error' && !blockedByValidation)
    const label = saveState === 'saving'
        ? 'Saving…'
        : saveState === 'saved'
            ? 'Saved'
            : saveState === 'conflict'
                ? 'Save conflict'
                : blockedByValidation
                    ? 'Not saved'
                    : saveState === 'error'
                        ? 'Save failed'
                        : null
    const detail = [blockedByValidation ? null : saveErrorMessage, remediation?.message].filter(Boolean).join(' ')
    const reload = saveState === 'conflict' && onReload

    return (
        <div role="status" data-testid="editor-save-status" className={cn('pointer-events-none', className)}>
            {failed ? (
                <div
                    title={detail || undefined}
                    className={cn(
                        'pointer-events-auto flex w-72 max-w-[calc(100vw-2rem)] flex-col gap-1.5 rounded-md border bg-background p-2.5 text-xs shadow-sm',
                        saveState === 'conflict' ? 'border-warning/50' : 'border-destructive/50',
                    )}
                >
                    <span className={cn('font-semibold', saveState === 'conflict' ? 'text-warning' : 'text-destructive')}>
                        {label}<span className="sr-only">: </span>
                    </span>
                    {detail ? <span className="line-clamp-4 break-words text-muted-foreground">{detail}</span> : null}
                    {remediation?.allowRetry || reload ? (
                        <span className="flex gap-2">
                            {remediation?.allowRetry ? (
                                <Button type="button" data-testid="editor-save-retry" size="xs" variant="outline" onClick={() => void retryLastSaveContent()}>
                                    Retry save
                                </Button>
                            ) : null}
                            {reload ? (
                                <Button type="button" data-testid="editor-save-reload" size="xs" variant="outline" onClick={onReload}>
                                    Reload flow
                                </Button>
                            ) : null}
                        </span>
                    ) : null}
                </div>
            ) : label ? (
                <div
                    title={detail || undefined}
                    className={cn(
                        'pointer-events-auto flex max-w-xs items-center gap-2 rounded-md border bg-background px-2 py-1 text-xs font-medium shadow-sm',
                        blockedByValidation ? 'text-warning'
                            : saveState === 'saved' ? 'text-success' : 'text-muted-foreground',
                    )}
                >
                    <span>
                        {label}
                        {detail ? <span className="sr-only">: {detail}</span> : null}
                    </span>
                </div>
            ) : null}
        </div>
    )
}
