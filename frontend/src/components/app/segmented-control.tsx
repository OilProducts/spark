import type { ComponentProps, ReactNode } from 'react'

import { cn } from '@/lib/utils'

export interface SegmentedOption extends Omit<ComponentProps<'button'>, 'children' | 'onClick'> {
    key: string
    label: ReactNode
    pressed: boolean
    onPress: () => void
    'data-testid'?: string
}

/** A row of toggle buttons on a track; the pressed one is a white chip. Each carries `aria-pressed`. */
export function SegmentedControl({ options, className, ...props }: ComponentProps<'div'> & { options: SegmentedOption[] }) {
    return (
        <div role="group" className={cn('flex gap-0.5 rounded-lg bg-border p-0.5', className)} {...props}>
            {options.map(({ key, label, pressed, onPress, className: optionClassName, ...option }) => (
                <button
                    key={key}
                    type="button"
                    aria-pressed={pressed}
                    onClick={onPress}
                    className={cn(
                        'inline-flex h-7 min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md px-2.5 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50',
                        pressed ? 'bg-background font-semibold text-foreground' : 'font-medium text-muted-foreground hover:text-foreground',
                        optionClassName,
                    )}
                    {...option}
                >
                    {label}
                </button>
            ))}
        </div>
    )
}
