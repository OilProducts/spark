import { useEffect, useRef, useState, type ComponentProps, type ComponentType, type ReactNode } from 'react'
import { PanelLeftClose, PanelLeftOpen } from 'lucide-react'

import { Button } from '@/components/ui/button'
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
 * A view's side panel (on the ground) beside its sheet, narrower below 1280px so the sheet keeps
 * room; on a desktop it can be hidden, leaving the sheet (a canvas or run graph) the width. On a
 * narrow viewport they stack in the view's one scroll, the panel at full length, and picking
 * something (`selected` changes) scrolls the sheet into view.
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
    const [panelHidden, setPanelHidden] = useState(false)
    const hidden = panelHidden && !isNarrowViewport
    // The hide and show buttons swap places; focus follows from one to the other.
    const panelToggleRef = useRef<HTMLButtonElement | null>(null)
    const togglingPanel = useRef(false)
    const togglePanel = (next: boolean) => {
        togglingPanel.current = true
        setPanelHidden(next)
    }
    useEffect(() => {
        if (togglingPanel.current) {
            togglingPanel.current = false
            panelToggleRef.current?.focus()
        }
    }, [panelHidden])
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
                data-hidden={hidden ? 'true' : undefined}
                aria-label={`${title} panel`}
                className={cn('flex shrink-0 flex-col', !isNarrowViewport && (hidden ? 'w-12 items-center' : 'w-60 xl:w-70'))}
            >
                {hidden ? (
                    <Button
                        ref={panelToggleRef}
                        data-testid="side-panel-show"
                        variant="ghost"
                        size="icon-sm"
                        className="mt-2.5 text-muted-foreground"
                        aria-label={`Show ${title} panel`}
                        title={`Show ${title} panel`}
                        onClick={() => togglePanel(false)}
                    >
                        <PanelLeftOpen />
                    </Button>
                ) : (
                    // Not a heading: the aside is named for it, and the sheet's h1 leads the page's outline.
                    <div className="flex items-center justify-between gap-2 px-3 pb-2 pt-3.5">
                        <p data-testid="side-panel-title" className="text-base font-semibold text-foreground">{title}</p>
                        <div className="flex items-center gap-1">
                            {actions}
                            {isNarrowViewport ? null : (
                                <Button
                                    ref={panelToggleRef}
                                    data-testid="side-panel-hide"
                                    variant="ghost"
                                    size="icon-sm"
                                    className="text-muted-foreground"
                                    aria-label={`Hide ${title} panel`}
                                    title={`Hide ${title} panel`}
                                    onClick={() => togglePanel(true)}
                                >
                                    <PanelLeftClose />
                                </Button>
                            )}
                        </div>
                    </div>
                )}
                {/* Hidden, not unmounted, so the list keeps its search, scroll and expanded folders. */}
                <div hidden={hidden} className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">{panel}</div>
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
