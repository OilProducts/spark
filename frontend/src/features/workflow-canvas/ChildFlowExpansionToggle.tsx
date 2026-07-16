import { Button } from '@/components/ui/button'
export function ChildFlowExpansionToggle({
    expanded,
    onChange,
    disabled = false,
    testId,
}: {
    expanded: boolean
    onChange: (expanded: boolean) => void
    disabled?: boolean
    testId?: string
}) {
    return (
        <div
            data-testid={testId}
            className="flex rounded-md border border-border bg-background/90 p-1 shadow-sm"
        >
            <Button
                type="button"
                size="xs"
                variant={expanded ? 'ghost' : 'default'}
                disabled={disabled}
                className={expanded ? 'text-muted-foreground hover:text-foreground' : ''}
                onClick={() => onChange(false)}
            >
                Parent only
            </Button>
            <Button
                type="button"
                size="xs"
                variant={expanded ? 'default' : 'ghost'}
                disabled={disabled}
                className={expanded ? '' : 'text-muted-foreground hover:text-foreground'}
                onClick={() => onChange(true)}
            >
                Child flows
            </Button>
        </div>
    )
}
