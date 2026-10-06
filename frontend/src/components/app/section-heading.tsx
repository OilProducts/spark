import type { ComponentProps, ReactNode } from 'react'

import { cn } from '@/lib/utils'

/**
 * A sentence-case section heading, at least as prominent as the text under it, with an
 * optional muted count and a trailing action slot.
 */
export function SectionHeading({
    as: Tag = 'h2',
    count,
    action,
    className,
    children,
    ...props
}: ComponentProps<'h2'> & { as?: 'h2' | 'h3' | 'h4'; count?: ReactNode; action?: ReactNode }) {
    return (
        <div className="flex items-center justify-between gap-3">
            <Tag className={cn('text-sm font-semibold text-foreground', className)} {...props}>
                {children}
                {count !== undefined && count !== null ? <span className="ml-1.5 text-sm font-normal text-muted-foreground">{count}</span> : null}
            </Tag>
            {action ? <div className="flex shrink-0 items-center gap-1">{action}</div> : null}
        </div>
    )
}
