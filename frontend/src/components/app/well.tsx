import type { ComponentProps } from 'react'

import { cn } from '@/lib/utils'

/** A group inside a sheet: muted background, 12px radius, no border. */
export function Well({ className, ...props }: ComponentProps<'div'>) {
    return <div data-slot="well" className={cn('rounded-xl bg-muted p-4', className)} {...props} />
}
