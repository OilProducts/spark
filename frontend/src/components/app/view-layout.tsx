import type { ComponentProps, ReactNode } from 'react'

import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { cn } from '@/lib/utils'

/**
 * The one white panel of a view, sitting on the ground: inset 8px from the window edge
 * with a hairline outline and 12px radius. `ViewLayout` renders its main area as a sheet
 * flush against the side panel; a view without a side panel (Overview) is wrapped in one
 * by the app shell. On a narrow viewport the sheet drops its inset and radius.
 */
export function Sheet({ className, ...props }: ComponentProps<'div'>) {
    const isNarrowViewport = useNarrowViewport()
    return (
        <div
            data-slot="sheet"
            className={cn(
                'relative min-h-0 min-w-0 flex-1 bg-background',
                isNarrowViewport ? 'border-t border-border' : 'm-2 overflow-hidden rounded-xl border border-border',
                className,
            )}
            {...props}
        />
    )
}

/** A view's side panel (on the ground) beside its sheet; on a narrow viewport they stack. */
export function ViewLayout({
    view,
    title,
    actions,
    panel,
    children,
}: {
    view: string
    title: string
    actions?: ReactNode
    panel: ReactNode
    children: ReactNode
}) {
    const isNarrowViewport = useNarrowViewport()
    return (
        <div
            data-testid={`${view}-view`}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`flex h-full min-h-0 ${isNarrowViewport ? 'flex-col overflow-y-auto' : 'flex-row'}`}
        >
            <aside
                data-testid="side-panel"
                aria-label={`${title} panel`}
                className={`flex shrink-0 flex-col ${isNarrowViewport ? 'max-h-[40vh]' : 'w-70'}`}
            >
                <div className="flex items-center justify-between gap-2 px-3 pb-2 pt-3.5">
                    <h2 data-testid="side-panel-title" className="text-base font-semibold text-foreground">{title}</h2>
                    {actions ? <div className="flex items-center gap-1">{actions}</div> : null}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">{panel}</div>
            </aside>
            <Sheet data-testid="view-main" className={isNarrowViewport ? 'min-h-[calc(100dvh-3.25rem)]' : 'ml-0'}>
                {children}
            </Sheet>
        </div>
    )
}
