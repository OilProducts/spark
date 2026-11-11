import { useEffect, useRef, type ComponentProps, type ComponentType, type ReactNode } from 'react'

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

/**
 * A view's side panel (on the ground) beside its sheet. On a narrow viewport they stack in the
 * view's one scroll, the panel at full length, and picking something (`selected` changes)
 * scrolls the sheet into view.
 */
export function ViewLayout({
    view,
    title,
    actions,
    panel,
    selected,
    children,
}: {
    view: string
    title: string
    actions?: ReactNode
    panel: ReactNode
    /** The key of what the sheet shows; null when nothing is picked. */
    selected?: string | null
    children: ReactNode
}) {
    const isNarrowViewport = useNarrowViewport()
    const sheetRef = useRef<HTMLDivElement | null>(null)
    useEffect(() => {
        if (isNarrowViewport && selected) {
            sheetRef.current?.scrollIntoView({ block: 'start' })
        }
    }, [isNarrowViewport, selected])
    return (
        <div
            data-testid={`${view}-view`}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`flex h-full min-h-0 ${isNarrowViewport ? 'flex-col overflow-y-auto' : 'flex-row'}`}
        >
            <aside
                data-testid="side-panel"
                aria-label={`${title} panel`}
                className={cn('flex shrink-0 flex-col', !isNarrowViewport && 'w-70')}
            >
                {/* Not a heading: the aside is named for it, and the sheet's h1 leads the page's outline. */}
                <div className="flex items-center justify-between gap-2 px-3 pb-2 pt-3.5">
                    <p data-testid="side-panel-title" className="text-base font-semibold text-foreground">{title}</p>
                    {actions ? <div className="flex items-center gap-1">{actions}</div> : null}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">{panel}</div>
            </aside>
            <Sheet ref={sheetRef} data-testid="view-main" className={isNarrowViewport ? 'min-h-[calc(100dvh-3.25rem)]' : 'ml-0'}>
                {children}
            </Sheet>
        </div>
    )
}

/**
 * What a view's sheet shows when nothing is selected or there is nothing yet: the view's
 * icon, a short title, one line of guidance and an optional action, centred in the sheet.
 */
export function SheetEmpty({
    icon: Icon,
    title,
    action,
    className,
    children,
    ...props
}: Omit<ComponentProps<'div'>, 'title'> & { icon?: ComponentType<{ className?: string }>; title: ReactNode; action?: ReactNode }) {
    return (
        <div data-slot="sheet-empty" className={cn('flex h-full min-h-64 items-center justify-center p-8', className)} {...props}>
            <div className="flex max-w-sm flex-col items-center gap-3 text-center">
                {Icon ? (
                    <span aria-hidden="true" className="flex size-10 items-center justify-center rounded-xl bg-muted text-muted-foreground">
                        <Icon className="size-5" />
                    </span>
                ) : null}
                <div className="space-y-1">
                    <p className="text-sm font-semibold text-foreground">{title}</p>
                    {children ? <div className="text-sm text-muted-foreground">{children}</div> : null}
                </div>
                {action}
            </div>
        </div>
    )
}
