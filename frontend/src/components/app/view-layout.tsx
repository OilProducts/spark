import type { ReactNode } from 'react'

import { useNarrowViewport } from '@/lib/useNarrowViewport'

/** A view's side panel beside its main area; on a narrow viewport they stack. */
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
                className={`flex shrink-0 flex-col bg-background ${isNarrowViewport
                    ? 'max-h-[40vh] border-b border-border'
                    : 'w-72 border-r border-border'}`}
            >
                <div className="flex items-baseline justify-between gap-2 px-4 pb-2 pt-4">
                    <h2 data-testid="side-panel-title" className="text-base font-normal text-foreground">{title}</h2>
                    {actions ? <div className="flex items-center gap-1">{actions}</div> : null}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto pb-4">{panel}</div>
            </aside>
            <div
                data-testid="view-main"
                className={`relative min-w-0 flex-1 ${isNarrowViewport ? 'min-h-[calc(100dvh-3rem)]' : 'min-h-0'}`}
            >
                {children}
            </div>
        </div>
    )
}
