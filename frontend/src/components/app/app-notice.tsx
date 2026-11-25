import { useState, type ComponentProps, type ReactNode } from 'react'
import { TriangleAlert, X } from 'lucide-react'

import { cn } from '@/lib/utils'

/**
 * A problem with the whole app, such as settings that couldn't load, as a banner above the open
 * view; several stack. Its close button hides it, telling `onDismiss`; give it a new `key` to show a new problem again.
 */
export function AppNotice({ title, actions, onDismiss, className, children, ...props }: ComponentProps<'div'> & { title?: ReactNode; actions?: ReactNode; onDismiss?: () => void }) {
    const [dismissed, setDismissed] = useState(false)
    if (dismissed) return null
    return (
        <div
            role="alert"
            className={cn('flex w-full gap-3 rounded-lg border border-border bg-background p-3 text-sm', className)}
            {...props}
        >
            <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />
            <div className="flex min-w-0 flex-1 flex-col gap-2">
                <div className="flex flex-col gap-0.5">
                    {title ? <p className="break-words font-medium text-foreground">{title}</p> : null}
                    <p className="break-words text-muted-foreground">{children}</p>
                </div>
                {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
            </div>
            <button
                type="button"
                aria-label="Dismiss"
                onClick={() => { setDismissed(true); onDismiss?.() }}
                className="-m-1 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
                <X aria-hidden="true" className="size-4" />
            </button>
        </div>
    )
}
