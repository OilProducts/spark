import type { ComponentProps, ReactNode } from 'react'
import { TriangleAlert } from 'lucide-react'

import { cn } from '@/lib/utils'

/**
 * A problem with the whole app, such as settings that couldn't load. It floats over the corner of
 * the screen so it never moves the view under it; several stack.
 */
export function AppNotice({ title, actions, className, children, ...props }: ComponentProps<'div'> & { title?: ReactNode; actions?: ReactNode }) {
    return (
        <div
            role="alert"
            className={cn('pointer-events-auto flex gap-3 rounded-lg border border-border bg-background p-3 text-sm shadow-lg', className)}
            {...props}
        >
            <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />
            <div className="flex min-w-0 flex-col gap-2">
                <div className="flex flex-col gap-0.5">
                    {title ? <p className="font-medium text-foreground">{title}</p> : null}
                    <p className="break-words text-muted-foreground">{children}</p>
                </div>
                {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
            </div>
        </div>
    )
}
