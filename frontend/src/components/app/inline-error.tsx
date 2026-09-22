import type { ComponentProps, ReactNode } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'

// The one inline error look. AlertDescription defaults to muted text, so it inherits the destructive color here.
export function InlineError({ title, dense, children, ...props }: ComponentProps<typeof Alert> & { title?: ReactNode, dense?: boolean }) {
    return (
        <Alert variant="destructive" role="alert" {...props}>
            {title ? <AlertTitle>{title}</AlertTitle> : null}
            <AlertDescription className={dense ? 'text-xs text-inherit' : 'text-inherit'}>{children}</AlertDescription>
        </Alert>
    )
}
