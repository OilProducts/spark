import type { ReactNode } from 'react'

interface InspectorScaffoldProps {
    scopeLabel: 'Graph' | 'Node' | 'Edge'
    title: string
    description: string
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
            className="rounded-md border border-border/80 bg-background/50 p-4"
        >
            <div className="space-y-2">
                <div className="flex items-center gap-2">
                    <span className="rounded border border-border bg-background px-2 py-0.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        {scopeLabel}
                    </span>
                    <h2 className="text-base font-semibold tracking-tight text-foreground">{title}</h2>
                </div>
                <p className="text-xs text-muted-foreground">{description}</p>
                {entityValue ? (
                    <div className="rounded border border-border/80 bg-muted/20 px-2 py-1 text-xs text-muted-foreground">
                        <span className="font-semibold text-foreground">{entityLabel || 'Selection'}:</span>{' '}
                        <span className="font-mono break-all">{entityValue}</span>
                    </div>
                ) : null}
            </div>
            <div className="mt-4 space-y-5">{children}</div>
        </section>
    )
}

export function InspectorEmptyState({ message }: InspectorEmptyStateProps) {
    return (
        <div
            data-testid="inspector-empty-state"
            className="flex min-h-40 items-center justify-center rounded-md border border-dashed border-border/80 bg-muted/20 px-4 text-center text-sm text-muted-foreground"
        >
            <p>{message}</p>
        </div>
    )
}
