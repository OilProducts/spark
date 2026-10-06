import type { ComponentProps } from 'react'

import { cn } from '@/lib/utils'

/**
 * One status vocabulary for marks. `danger`/`destructive` and `neutral`/`muted` are
 * synonyms so the transcript tones (SurfaceTone) and the runs tones (StatusTone in
 * features/runs/model/shared.ts) both fit without a mapping step.
 */
export type StatusTone = 'neutral' | 'muted' | 'info' | 'success' | 'warning' | 'danger' | 'destructive'

const TEXT: Record<StatusTone, string> = {
    neutral: 'text-muted-foreground',
    muted: 'text-muted-foreground',
    info: 'text-info',
    success: 'text-success',
    warning: 'text-warning',
    danger: 'text-destructive',
    destructive: 'text-destructive',
}

const DOT: Record<StatusTone, string> = {
    neutral: 'bg-muted-foreground/60',
    muted: 'bg-muted-foreground/60',
    info: 'bg-info',
    success: 'bg-success',
    warning: 'bg-warning',
    danger: 'bg-destructive',
    destructive: 'bg-destructive',
}

const PILL: Record<StatusTone, string> = {
    neutral: 'bg-muted text-muted-foreground',
    muted: 'bg-muted text-muted-foreground',
    info: 'bg-info/10 text-info',
    success: 'bg-success/10 text-success',
    warning: 'bg-warning/10 text-warning',
    danger: 'bg-destructive/10 text-destructive',
    destructive: 'bg-destructive/10 text-destructive',
}

/** The text colour for a tone, for a word or icon that marks status inline. */
export const statusTextClassName = (tone: StatusTone) => TEXT[tone]

/** A 7px dot leading a row. `hollow` draws a ring instead (drafts). */
export function StatusDot({ tone, hollow = false, className, ...props }: ComponentProps<'span'> & { tone: StatusTone; hollow?: boolean }) {
    return (
        <span
            aria-hidden="true"
            data-slot="status-dot"
            data-tone={tone}
            className={cn(
                'inline-block size-[7px] shrink-0 rounded-full',
                hollow ? 'border border-muted-foreground/70' : DOT[tone],
                className,
            )}
            {...props}
        />
    )
}

/** Dot plus word on a tinted pill. Once per screen, on the item's own header. */
export function StatusPill({ tone, className, children, ...props }: ComponentProps<'span'> & { tone: StatusTone }) {
    return (
        <span
            data-slot="status-pill"
            data-tone={tone}
            className={cn('inline-flex h-6 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 text-xs font-medium', PILL[tone], className)}
            {...props}
        >
            <StatusDot tone={tone} className="size-1.5" />
            {children}
        </span>
    )
}
