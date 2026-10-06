import type { ComponentProps, ReactNode } from 'react'

import { cn } from '@/lib/utils'
import { StatusDot, type StatusTone } from './status'

/** A sentence-case group label above side-panel rows ("Today", "Drafts · 3"). */
export function ListGroupLabel({ className, ...props }: ComponentProps<'div'>) {
    return <div className={cn('px-3 pb-1 pt-3 text-xs font-semibold text-muted-foreground first:pt-0', className)} {...props} />
}

/**
 * One row in a side panel: a status dot, a title line, a meta line and an optional
 * trailing slot. Selected = a white chip on the ground. Renders a button and sets
 * `aria-current="true"` when selected.
 */
export function SidePanelRow({
    tone,
    hollow = false,
    title,
    titleLines = 1,
    meta,
    trailing,
    selected = false,
    className,
    ...props
}: Omit<ComponentProps<'button'>, 'title'> & {
    tone?: StatusTone
    hollow?: boolean
    title: ReactNode
    titleLines?: 1 | 2
    meta?: ReactNode
    trailing?: ReactNode
    selected?: boolean
}) {
    return (
        <button
            type="button"
            aria-current={selected ? 'true' : undefined}
            className={cn(
                'flex w-full min-w-0 items-start gap-2.5 rounded-lg border px-2.5 py-2 text-left text-foreground outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
                selected ? 'border-border bg-background' : 'border-transparent hover:bg-background/60',
                className,
            )}
            {...props}
        >
            {tone ? <StatusDot tone={tone} hollow={hollow} className="mt-[7px]" /> : null}
            <span className="flex min-w-0 flex-1 flex-col gap-px">
                <span className={cn('text-sm font-medium leading-5', titleLines === 2 ? 'line-clamp-2' : 'truncate')}>{title}</span>
                {meta ? <span className="truncate text-xs text-muted-foreground">{meta}</span> : null}
            </span>
            {trailing ? <span className="mt-0.5 flex shrink-0 items-center gap-1 text-xs text-muted-foreground">{trailing}</span> : null}
        </button>
    )
}
