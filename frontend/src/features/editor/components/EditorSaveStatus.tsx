import { useEffect } from 'react'

import { Button } from '@/components/ui/button'
import { retryLastSaveContent } from '@/lib/flowPersistence'
import { resolveSaveRemediation } from '@/lib/saveRemediation'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'
import { flowProblems } from '../model/flowLoadFailure'

const SAVED_SHOWN_MS = 2000

/**
 * The flow's save state, below the toolbar so it never covers or moves the actions it gates:
 * "Saving…", a brief "Saved", or why the edits aren't saved yet, in a corner of the canvas. A failed
 * save is a strip above the canvas, so it covers no step: its reason and the way out as text and
 * buttons, not only on hover: Retry when trying again can help, and Reload flow after a conflict.
 */
export function EditorSaveStatus({ className, onReload }: { className?: string; onReload?: () => void }) {
    const saveState = useStore((state) => state.saveState)
    const saveStateVersion = useStore((state) => state.saveStateVersion)
    const saveErrorMessage = useStore((state) => state.saveErrorMessage)
    const saveErrorKind = useStore((state) => state.saveErrorKind)
    const saveErrorProblems = useStore((state) => state.saveErrorProblems)
    const hasValidationErrors = useStore((state) => state.hasValidationErrors)
    const resetSaveState = useStore((state) => state.resetSaveState)

    // "Saved" shows briefly, then clears.
    useEffect(() => {
        if (saveState !== 'saved') return
        const timer = window.setTimeout(resetSaveState, SAVED_SHOWN_MS)
        return () => window.clearTimeout(timer)
    }, [resetSaveState, saveState, saveStateVersion])

    const refused = saveState === 'error' && saveErrorKind === 'validation_error'
    // A flow saves only once it validates. The Validation panel lists the errors it knows of; a
    // refusal for one it doesn't list shows the reason here instead.
    const blockedByValidation = refused && hasValidationErrors
    const remediation = refused && !blockedByValidation ? null : resolveSaveRemediation(saveState, saveErrorKind)
    const failed = saveState === 'conflict' || (saveState === 'error' && !blockedByValidation)
    const label = saveState === 'saving'
        ? 'Saving…'
        : saveState === 'saved'
            ? 'Saved'
            : saveState === 'conflict'
                ? 'Save conflict'
                : refused
                    ? 'Not saved'
                    : saveState === 'error'
                        ? 'Save failed'
                        : null
    const reason = refused && saveErrorMessage
        ? (saveErrorProblems?.map((problem) => problem.message) ?? flowProblems(saveErrorMessage)).join(' ')
        : saveErrorMessage
    const detail = [blockedByValidation ? null : reason, remediation?.message].filter(Boolean).join(' ')
    const reload = saveState === 'conflict' && onReload

    return (
        <div role="status" data-testid="editor-save-status" className={cn('relative z-10', className)}>
            {failed ? (
                <div
                    title={detail || undefined}
                    className={cn(
                        'flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-2 text-xs',
                        saveState === 'conflict' ? 'bg-warning/10' : 'bg-destructive/5',
                    )}
                >
                    <span className={cn('font-semibold', saveState === 'conflict' ? 'text-warning' : 'text-destructive')}>
                        {label}<span className="sr-only">: </span>
                    </span>
                    <span className="line-clamp-2 min-w-0 flex-1 break-words text-muted-foreground">{detail}</span>
                    {remediation?.allowRetry || reload ? (
                        <span className="flex shrink-0 gap-2">
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
                        'absolute right-3 top-3 flex max-w-xs items-center gap-2 rounded-md border bg-background px-2 py-1 text-xs font-medium shadow-sm',
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
