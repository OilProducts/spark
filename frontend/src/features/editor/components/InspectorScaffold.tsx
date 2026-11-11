import type { ReactNode } from 'react'
import { Well } from '@/components/app/well'

interface InspectorScaffoldProps {
    scopeLabel: 'Graph' | 'Node' | 'Edge'
    title: string
    description?: string
    entityLabel?: string
    entityValue?: string
    children: ReactNode
}

interface InspectorEmptyStateProps {
    message: string
}

export function InspectorScaffold({
    scopeLabel,
    title,
    description,
    entityLabel,
    entityValue,
    children,
}: InspectorScaffoldProps) {
    return (
        <section
            data-testid="inspector-scaffold"
            data-inspector-scope={scopeLabel.toLowerCase()}
        >
            <div className="space-y-2">
                <h2 className="text-base font-semibold text-foreground">{title}</h2>
                {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
                {entityValue ? (
                    <div className="rounded-lg bg-muted px-2.5 py-1.5 text-xs text-muted-foreground">
                        <span className="font-semibold text-foreground">{entityLabel || 'Selection'}:</span>{' '}
                        {/* Paths wrap after a slash, not mid-word. */}
                        <span className="font-mono break-words">
                            {entityValue.split('/').map((part, index) => (
                                <span key={index}>{index > 0 ? <>/<wbr /></> : null}{part}</span>
                            ))}
                        </span>
                    </div>
                ) : null}
            </div>
            <div className="mt-4 space-y-5">{children}</div>
        </section>
    )
}

export function InspectorEmptyState({ message }: InspectorEmptyStateProps) {
    return (
        <Well
            data-testid="inspector-empty-state"
            className="flex min-h-40 items-center justify-center px-4 text-center text-sm text-muted-foreground"
        >
            <p>{message}</p>
        </Well>
    )
}
